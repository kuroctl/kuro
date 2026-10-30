//! Ressources à chaud, quota disque et suspension (ADR-0019).

use std::collections::BTreeMap;
use std::time::Duration;

use kuro_daemon::docker::container_name;
use kuro_daemon::fs::ServerDir;
use kuro_daemon::server::CommandError;
use kuro_protocol::DaemonMessage;
use kuro_protocol::api::backup::BackupCreate;
use kuro_protocol::api::install::{InstallMode, InstallSpec};
use kuro_protocol::api::server::{Limits, PowerAction, ServerLimits};
use kuro_protocol::daemon::server::ServerState;
use tokio::sync::mpsc;
use uuid::Uuid;

use super::support::{IMAGE, Node, deploy_and_start, node, single, wait_for};

const MIB: u64 = 1024 * 1024;

fn bollard() -> bollard::Docker {
    bollard::Docker::connect_with_defaults().unwrap()
}

/// Attend un `server.disk` du serveur qui vérifie `accept` ; retourne l'espace rapporté.
async fn disk_report(
    streams: &mut mpsc::Receiver<DaemonMessage>,
    server_id: Uuid,
    accept: impl Fn(u64) -> bool,
) -> u64 {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match streams.recv().await.expect("streams closed") {
                DaemonMessage::ServerDisk(disk)
                    if disk.server_id == server_id && accept(disk.used_bytes) =>
                {
                    return disk.used_bytes;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("no matching disk report")
}

async fn write(node: &Node, id: Uuid, path: &str, bytes: usize) -> Result<(), CommandError> {
    node.manager
        .file_write(id, path, "x".repeat(bytes), None)
        .await
}

/// Arrête et supprime. Une mise à jour des limites peut encore tenir le serveur : on attend.
async fn cleanup(node: &mut Node, id: Uuid) {
    for attempt in 0.. {
        match node.manager.power(id, PowerAction::Kill).await {
            Ok(()) => break,
            Err(CommandError::Busy) if attempt < 50 => {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(error) => panic!("kill failed: {error}"),
        }
    }
    wait_for(&mut node.events, id, ServerState::Offline).await;
    node.manager.delete(id).unwrap();
    wait_for(&mut node.events, id, ServerState::Deleted).await;
}

#[tokio::test]
async fn limits_apply_live_without_restarting_the_server() {
    let mut node = node();
    let id = Uuid::now_v7();
    deploy_and_start(&mut node, single(id)).await;
    let main = container_name(id, "app");
    let started = bollard()
        .inspect_container(&main, None)
        .await
        .unwrap()
        .state
        .unwrap()
        .started_at;

    node.manager
        .limits(ServerLimits {
            server_id: id,
            services: BTreeMap::from([(
                "app".to_owned(),
                Limits {
                    memory_bytes: 128 * MIB,
                    cpu_millis: 1000,
                    pids: 64,
                },
            )]),
            disk_bytes: 2048 * MIB,
        })
        .unwrap();

    let mut host = None;
    for _ in 0..100 {
        let inspect = bollard().inspect_container(&main, None).await.unwrap();
        let config = inspect.host_config.unwrap();
        if config.memory == Some(128 * 1024 * 1024) {
            host = Some((config, inspect.state.unwrap()));
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let (config, state) = host.expect("limits not applied");
    assert_eq!(config.nano_cpus, Some(1_000_000_000));
    assert_eq!(state.running, Some(true));
    assert_eq!(state.started_at, started, "the server was restarted");
    // Gardées pour les prochaines créations de conteneurs.
    let saved = ServerDir::new(node.data.path(), id)
        .load_spec()
        .unwrap()
        .unwrap();
    assert_eq!(saved.services[0].limits.memory_bytes, 128 * MIB);
    assert_eq!(saved.disk_bytes, 2048 * MIB);

    // Limites refusées par les règles du daemon (mémoire trop basse), ou service inconnu.
    for services in [
        BTreeMap::from([(
            "app".to_owned(),
            Limits {
                memory_bytes: 1024,
                cpu_millis: 1000,
                pids: 64,
            },
        )]),
        BTreeMap::new(),
    ] {
        let refused = node.manager.limits(ServerLimits {
            server_id: id,
            services,
            disk_bytes: 0,
        });
        assert!(
            matches!(refused, Err(CommandError::InvalidSpec(_))),
            "{refused:?}"
        );
    }
    cleanup(&mut node, id).await;
}

#[tokio::test]
async fn a_suspended_server_is_stopped_and_refuses_everything_but_stop_and_delete() {
    let mut node = node();
    let id = Uuid::now_v7();
    deploy_and_start(&mut node, single(id)).await;

    node.manager.suspend(id, true).unwrap();
    wait_for(&mut node.events, id, ServerState::Offline).await;

    let dir = ServerDir::new(node.data.path(), id);
    assert!(dir.load_spec().unwrap().unwrap().suspended);
    assert_eq!(
        node.manager.power(id, PowerAction::Start).await,
        Err(CommandError::Suspended)
    );
    assert_eq!(
        node.manager.power(id, PowerAction::Restart).await,
        Err(CommandError::Suspended)
    );
    assert_eq!(
        node.manager.command(id, "say hi").await,
        Err(CommandError::Suspended)
    );
    assert_eq!(
        node.manager.file_list(id, "/app/data").await.map(|_| ()),
        Err(CommandError::Suspended)
    );
    assert_eq!(
        write(&node, id, "/app/data/a.txt", 10).await,
        Err(CommandError::Suspended)
    );
    let backup = BackupCreate {
        server_id: id,
        backup_id: Uuid::now_v7(),
        ignore: Vec::new(),
        before: Vec::new(),
        after: Vec::new(),
    };
    assert_eq!(
        node.manager.backup_create(backup).await,
        Err(CommandError::Suspended)
    );
    let install = InstallSpec {
        service: "app".to_owned(),
        image: IMAGE.to_owned(),
        steps: Vec::new(),
    };
    assert_eq!(
        node.manager
            .install(id, InstallMode::Reinstall, Some(install))
            .await,
        Err(CommandError::Suspended)
    );

    node.manager.suspend(id, false).unwrap();
    assert!(!dir.load_spec().unwrap().unwrap().suspended);
    node.manager.power(id, PowerAction::Start).await.unwrap();
    wait_for(&mut node.events, id, ServerState::Running).await;
    cleanup(&mut node, id).await;
}

#[tokio::test]
async fn disk_usage_counts_volumes_and_backups() {
    let mut node = node();
    let id = Uuid::now_v7();
    node.manager.deploy(single(id)).unwrap();
    wait_for(&mut node.events, id, ServerState::Offline).await;
    // Une archive de sauvegarde, dans le dossier du daemon.
    let backups = node.data.path().join(id.to_string()).join("backups");
    std::fs::create_dir_all(&backups).unwrap();
    std::fs::write(backups.join("old.tar.zst"), vec![0_u8; 5000]).unwrap();

    write(&node, id, "/app/data/level.dat", 1000).await.unwrap();

    let used = disk_report(&mut node.streams, id, |used| used >= 6000).await;
    assert_eq!(used, 6000);
    node.manager.delete(id).unwrap();
    wait_for(&mut node.events, id, ServerState::Deleted).await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_link_out_of_the_volume_is_not_counted() {
    let mut node = node();
    let id = Uuid::now_v7();
    node.manager.deploy(single(id)).unwrap();
    wait_for(&mut node.events, id, ServerState::Offline).await;
    let outside = tempfile::TempDir::new().unwrap();
    std::fs::write(outside.path().join("big"), vec![0_u8; 100_000]).unwrap();
    let volume = node
        .data
        .path()
        .join(id.to_string())
        .join("volumes/app/data");
    std::os::unix::fs::symlink(outside.path(), volume.join("escape")).unwrap();

    write(&node, id, "/app/data/a.txt", 10).await.unwrap();

    let used = disk_report(&mut node.streams, id, |used| used >= 10).await;
    assert_eq!(used, 10);
    node.manager.delete(id).unwrap();
    wait_for(&mut node.events, id, ServerState::Deleted).await;
}

#[tokio::test]
async fn over_the_quota_writes_backups_and_start_are_refused() {
    let mut node = node();
    let id = Uuid::now_v7();
    let mut spec = single(id);
    spec.disk_bytes = 4096;
    node.manager.deploy(spec).unwrap();
    wait_for(&mut node.events, id, ServerState::Offline).await;

    write(&node, id, "/app/data/a.txt", 3000).await.unwrap();
    disk_report(&mut node.streams, id, |used| used == 3000).await;
    assert_eq!(
        write(&node, id, "/app/data/b.txt", 2000).await,
        Err(CommandError::DiskQuotaExceeded)
    );
    // Juste le quota.
    write(&node, id, "/app/data/b.txt", 1096).await.unwrap();
    disk_report(&mut node.streams, id, |used| used == 4096).await;

    assert_eq!(
        node.manager.power(id, PowerAction::Start).await,
        Err(CommandError::DiskQuotaExceeded)
    );
    assert_eq!(
        node.manager.file_mkdir(id, "/app/data/more").await,
        Err(CommandError::DiskQuotaExceeded)
    );
    let backup = BackupCreate {
        server_id: id,
        backup_id: Uuid::now_v7(),
        ignore: Vec::new(),
        before: Vec::new(),
        after: Vec::new(),
    };
    assert_eq!(
        node.manager.backup_create(backup).await,
        Err(CommandError::DiskQuotaExceeded)
    );

    // Supprimer libère de la place.
    node.manager
        .file_delete(id, "/app/data/a.txt")
        .await
        .unwrap();
    disk_report(&mut node.streams, id, |used| used == 1096).await;
    write(&node, id, "/app/data/c.txt", 100).await.unwrap();
    node.manager.delete(id).unwrap();
    wait_for(&mut node.events, id, ServerState::Deleted).await;
}
