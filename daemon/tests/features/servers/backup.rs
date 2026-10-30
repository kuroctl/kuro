//! Sauvegardes (ADR-0018) : archive des volumes, motifs exclus, liens jamais suivis, restauration
//! (écraser ou tout effacer), lecture par morceaux, suppression, commandes autour de l'archivage.

use std::path::PathBuf;
use std::time::Duration;

use base64::Engine;
use kuro_daemon::backup::pattern::Ignore;
use kuro_daemon::docker::Docker;
use kuro_daemon::fs::ServerDir;
use kuro_daemon::server::{CommandError, Manager};
use kuro_protocol::api::{BackupCreate, BackupHook, RestoreMode};
use kuro_protocol::daemon::server::ServerState;
use kuro_protocol::daemon::{BackupDone, BackupOutcome, BackupState};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::sync::mpsc;
use uuid::Uuid;

use super::support::{IMAGE, deploy_and_start, service, single, wait_for};

struct Node {
    manager: Manager,
    events: kuro_daemon::server::StateReceiver,
    backups: mpsc::Receiver<BackupDone>,
    data: TempDir,
    id: Uuid,
}

impl Node {
    /// Serveur déployé (spécification et volumes) sans conteneur.
    fn new() -> Self {
        let data = TempDir::new().unwrap();
        let docker = Docker::connect(IMAGE).unwrap();
        let (manager, outgoing) = Manager::new(docker, data.path().to_path_buf());
        let id = Uuid::now_v7();
        let dir = ServerDir::new(data.path(), id);
        let spec = single(id);
        dir.save_spec(&spec).unwrap();
        dir.ensure_volumes(&spec).unwrap();
        Self {
            manager,
            events: outgoing.events,
            backups: outgoing.backups,
            data,
            id,
        }
    }

    fn host(&self, relative: &str) -> PathBuf {
        self.data
            .path()
            .join(self.id.to_string())
            .join("volumes/app/data")
            .join(relative)
    }

    fn write(&self, relative: &str, content: &str) {
        let path = self.host(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn read(&self, relative: &str) -> Option<String> {
        std::fs::read_to_string(self.host(relative)).ok()
    }

    /// Sauvegarde et attend sa fin.
    async fn backup(&mut self, ignore: &[&str]) -> (Uuid, BackupOutcome) {
        let backup_id = Uuid::now_v7();
        self.manager
            .backup_create(BackupCreate {
                server_id: self.id,
                backup_id,
                ignore: ignore.iter().map(|p| (*p).to_owned()).collect(),
                before: Vec::new(),
                after: Vec::new(),
            })
            .await
            .unwrap();
        (backup_id, done(&mut self.backups, backup_id).await)
    }

    /// Restaure et attend la fin : `Ok` si `offline`, sinon la cause.
    async fn restore(&mut self, backup_id: Uuid, mode: RestoreMode) -> Result<(), String> {
        self.manager
            .backup_restore(self.id, backup_id, mode)
            .await
            .map_err(|e| e.to_string())?;
        let mut restoring = false;
        tokio::time::timeout(Duration::from_secs(120), async {
            while let Some(report) = self.events.recv().await {
                match report.state {
                    ServerState::Restoring => restoring = true,
                    ServerState::Offline if restoring => return Ok(()),
                    ServerState::Failed => return Err(report.reason.unwrap_or_default()),
                    _ => {}
                }
            }
            Err("event channel closed".to_owned())
        })
        .await
        .unwrap()
    }
}

async fn done(backups: &mut mpsc::Receiver<BackupDone>, backup_id: Uuid) -> BackupOutcome {
    tokio::time::timeout(Duration::from_secs(120), async {
        while let Some(done) = backups.recv().await {
            if done.backup_id == backup_id {
                return done.outcome;
            }
        }
        panic!("backup channel closed");
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn a_backup_restores_the_volumes_as_they_were() {
    let mut node = Node::new();
    node.write("world/level.dat", "level 1");
    node.write("server.properties", "motd=hello");
    node.write("logs/latest.log", "noise");

    let (backup_id, outcome) = node.backup(&["app/data/logs/**"]).await;
    let BackupOutcome::Ok { size, sha256 } = outcome else {
        panic!("backup failed: {outcome:?}");
    };
    assert!(size > 0);
    assert_eq!(sha256.len(), 64);

    node.write("world/level.dat", "level 2");
    std::fs::remove_file(node.host("server.properties")).unwrap();
    node.write("added.txt", "after the backup");

    node.restore(backup_id, RestoreMode::Clean).await.unwrap();

    assert_eq!(node.read("world/level.dat").as_deref(), Some("level 1"));
    assert_eq!(
        node.read("server.properties").as_deref(),
        Some("motd=hello")
    );
    // Tout effacé avant : ni l'ajout ni le dossier exclu ne restent.
    assert_eq!(node.read("added.txt"), None);
    assert_eq!(node.read("logs/latest.log"), None);
    // En root (production, CI), tout ce qui est restauré revient à l'utilisateur du service.
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let probe = node.data.path().join("probe");
        std::fs::write(&probe, "").unwrap();
        if std::fs::metadata(&probe).unwrap().uid() == 0 {
            for restored in ["world", "world/level.dat", "server.properties"] {
                let meta = std::fs::metadata(node.host(restored)).unwrap();
                assert_eq!((meta.uid(), meta.gid()), (1000, 1000), "{restored}");
            }
        }
    }
}

#[tokio::test]
async fn overwriting_keeps_files_added_since() {
    let mut node = Node::new();
    node.write("world/level.dat", "level 1");
    let (backup_id, _) = node.backup(&[]).await;
    node.write("world/level.dat", "level 2");
    node.write("added.txt", "after the backup");

    node.restore(backup_id, RestoreMode::Overwrite)
        .await
        .unwrap();

    assert_eq!(node.read("world/level.dat").as_deref(), Some("level 1"));
    assert_eq!(node.read("added.txt").as_deref(), Some("after the backup"));
}

#[cfg(unix)]
#[tokio::test]
async fn links_are_never_followed_nor_archived() {
    let mut node = Node::new();
    let secret = node.data.path().join("host-secret");
    std::fs::write(&secret, "host only").unwrap();
    std::os::unix::fs::symlink(&secret, node.host("escape")).unwrap();
    node.write("kept.txt", "kept");

    let (backup_id, _) = node.backup(&[]).await;
    std::fs::remove_file(node.host("escape")).unwrap();
    node.restore(backup_id, RestoreMode::Clean).await.unwrap();

    assert_eq!(node.read("kept.txt").as_deref(), Some("kept"));
    assert!(std::fs::symlink_metadata(node.host("escape")).is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn executables_stay_executable() {
    use std::os::unix::fs::PermissionsExt;
    let mut node = Node::new();
    node.write("run.sh", "#!/bin/sh");
    std::fs::set_permissions(node.host("run.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();

    let (backup_id, _) = node.backup(&[]).await;
    node.restore(backup_id, RestoreMode::Clean).await.unwrap();

    let mode = std::fs::metadata(node.host("run.sh"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o111, 0o111);
}

#[tokio::test]
async fn chunks_rebuild_the_archive_and_its_checksum() {
    let mut node = Node::new();
    // Contenu peu compressible, sur plusieurs morceaux.
    let mut noise = String::new();
    let mut state: u64 = 42;
    while noise.len() < 1_500_000 {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        noise.push(char::from(b'!' + u8::try_from(state >> 58).unwrap()));
    }
    node.write("noise.txt", &noise);
    let (backup_id, outcome) = node.backup(&[]).await;
    let BackupOutcome::Ok { size, sha256 } = outcome else {
        panic!("backup failed");
    };

    let mut archive = Vec::new();
    let mut chunks = 0;
    loop {
        let chunk = node
            .manager
            .backup_read(node.id, backup_id, archive.len() as u64)
            .await
            .unwrap();
        archive.extend(
            base64::engine::general_purpose::STANDARD
                .decode(chunk.data)
                .unwrap(),
        );
        chunks += 1;
        if chunk.eof {
            break;
        }
    }

    assert!(chunks > 1);
    assert_eq!(archive.len() as u64, size);
    let actual: String = Sha256::digest(&archive)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(actual, sha256);
}

#[tokio::test]
async fn listed_backups_can_be_deleted() {
    let mut node = Node::new();
    node.write("a.txt", "a");
    let (first, _) = node.backup(&[]).await;
    let (second, _) = node.backup(&[]).await;

    let listing = node.manager.backup_list(node.id).await.unwrap();
    let mut ids: Vec<Uuid> = listing.backups.iter().map(|b| b.backup_id).collect();
    ids.sort();
    assert_eq!(ids, [first, second]);
    assert!(
        listing
            .backups
            .iter()
            .all(|b| matches!(b.state, BackupState::Ready { .. }))
    );

    node.manager.backup_delete(node.id, first).await.unwrap();
    // Supprimer deux fois n'est pas une erreur.
    node.manager.backup_delete(node.id, first).await.unwrap();

    let listing = node.manager.backup_list(node.id).await.unwrap();
    assert_eq!(listing.backups.len(), 1);
    assert_eq!(listing.backups[0].backup_id, second);
    assert_eq!(
        node.manager
            .backup_restore(node.id, first, RestoreMode::Clean)
            .await
            .unwrap_err(),
        CommandError::NotFound
    );
    assert_eq!(
        node.manager
            .backup_read(node.id, first, 0)
            .await
            .unwrap_err(),
        CommandError::NotFound
    );
}

#[tokio::test]
async fn interrupted_archives_are_cleaned_up() {
    let node = Node::new();
    let backups = node.data.path().join(node.id.to_string()).join("backups");
    std::fs::create_dir_all(&backups).unwrap();
    let orphan = Uuid::now_v7();
    std::fs::write(
        backups.join(format!(".kuro-tmp-{orphan}.tar.zst")),
        "partial",
    )
    .unwrap();
    std::fs::write(backups.join(format!("{orphan}.tar.zst")), "no metadata").unwrap();

    let listing = node.manager.backup_list(node.id).await.unwrap();

    assert!(listing.backups.is_empty());
    assert_eq!(std::fs::read_dir(&backups).unwrap().count(), 0);
}

#[test]
fn ignore_patterns_cover_segments_and_depths() {
    let rules = Ignore::new(&[
        "app/data/logs/**".to_owned(),
        "app/data/*.log".to_owned(),
        "app/data/**/cache-*".to_owned(),
    ])
    .unwrap();

    assert!(rules.matches(&["app", "data", "logs"]));
    assert!(rules.matches(&["app", "data", "logs", "a", "latest.log"]));
    assert!(!rules.matches(&["app", "data", "logsx"]));
    assert!(rules.matches(&["app", "data", "debug.log"]));
    assert!(!rules.matches(&["app", "data", "world", "debug.log"]));
    assert!(rules.matches(&["app", "data", "a", "b", "cache-01"]));
    assert!(!rules.matches(&["app", "data", "cache"]));
    assert!(Ignore::new(&["a/**b".to_owned()]).is_none());
}

#[tokio::test]
async fn invalid_requests_are_refused() {
    let node = Node::new();
    let create = |ignore: &str, wait: Option<&str>| BackupCreate {
        server_id: node.id,
        backup_id: Uuid::now_v7(),
        ignore: vec![ignore.to_owned()],
        before: vec![BackupHook {
            console: "save".to_owned(),
            wait: wait.map(str::to_owned),
        }],
        after: Vec::new(),
    };

    for request in [create("a/**b", None), create("app/data", Some("(unclosed"))] {
        assert!(matches!(
            node.manager.backup_create(request).await,
            Err(CommandError::InvalidSpec(_))
        ));
    }
    assert_eq!(
        node.manager
            .backup_restore(Uuid::now_v7(), Uuid::now_v7(), RestoreMode::Clean)
            .await
            .unwrap_err(),
        CommandError::UnknownServer
    );
}

/// Les commandes `before` passent avant l'archivage : le fichier qu'elles font écrire au jeu est
/// dans la sauvegarde. Les commandes `after` suivent.
#[tokio::test]
async fn hooks_run_around_the_archive_of_a_running_server() {
    let mut node = super::support::node();
    let id = Uuid::now_v7();
    let mut spec = single(id);
    spec.files.clear();
    spec.services = vec![service(
        "app",
        "echo ready; while read line; do          if [ \"$line\" = save ]; then echo saved > /data/saved.txt; echo 'Saved the game'; fi;          if [ \"$line\" = resume ]; then echo resumed > /data/resumed.txt; fi; done",
    )];
    deploy_and_start(&mut node, spec).await;
    let backup_id = Uuid::now_v7();

    node.manager
        .backup_create(BackupCreate {
            server_id: id,
            backup_id,
            ignore: Vec::new(),
            before: vec![BackupHook {
                console: "save".to_owned(),
                wait: Some("^Saved the game$".to_owned()),
            }],
            after: vec![BackupHook {
                console: "resume".to_owned(),
                wait: None,
            }],
        })
        .await
        .unwrap();
    let outcome = done(&mut node.backups, backup_id).await;
    assert!(matches!(outcome, BackupOutcome::Ok { .. }), "{outcome:?}");

    let volume = node
        .data
        .path()
        .join(id.to_string())
        .join("volumes/app/data");
    std::fs::remove_file(volume.join("saved.txt")).unwrap();
    node.manager
        .backup_restore(id, backup_id, RestoreMode::Overwrite)
        .await
        .unwrap();
    wait_for(&mut node.events, id, ServerState::Restoring).await;
    wait_for(&mut node.events, id, ServerState::Offline).await;

    assert_eq!(
        std::fs::read_to_string(volume.join("saved.txt")).unwrap(),
        "saved
"
    );
    assert!(volume.join("resumed.txt").exists());
    node.manager.delete(id).unwrap();
    wait_for(&mut node.events, id, ServerState::Deleted).await;
}
