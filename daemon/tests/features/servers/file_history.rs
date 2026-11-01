//! Historique des fichiers (ADR-0049) par le chemin du panel : chaque écriture, suppression ou
//! restauration garde la version remplacée ; une restauration s'annule ; rien n'est visible
//! dans les volumes.

use kuro_daemon::docker::Docker;
use kuro_daemon::fs::ServerDir;
use kuro_daemon::server::{CommandError, Manager};
use kuro_protocol::api::{AuthorKind, FileAuthor};
use kuro_protocol::daemon::{VersionChannel, VersionReason};
use tempfile::TempDir;
use uuid::Uuid;

use super::support::{IMAGE, single};

const PATH: &str = "/app/data/server.properties";

struct Node {
    manager: Manager,
    data: TempDir,
    id: Uuid,
}

fn author() -> Option<FileAuthor> {
    Some(FileAuthor {
        kind: AuthorKind::Admin,
        id: Some(Uuid::from_u128(7)),
    })
}

impl Node {
    fn new() -> Self {
        let data = TempDir::new().unwrap();
        let docker = Docker::connect(IMAGE).unwrap();
        let (manager, _) = Manager::new(docker, data.path().to_path_buf());
        let id = Uuid::now_v7();
        let dir = ServerDir::new(data.path(), id);
        let spec = single(id);
        dir.save_spec(&spec).unwrap();
        dir.ensure_volumes(&spec).unwrap();
        Self { manager, data, id }
    }

    async fn write(&self, content: &str) {
        self.manager
            .file_write_as(self.id, PATH, content.to_owned(), None, author())
            .await
            .unwrap();
    }

    async fn current(&self) -> String {
        self.manager.file_read(self.id, PATH).await.unwrap().content
    }
}

#[tokio::test]
async fn each_write_keeps_the_version_it_replaces() {
    let node = Node::new();
    node.write("motd=one\n").await;
    node.write("motd=two\n").await;
    node.write("motd=three\n").await;

    let versions = node
        .manager
        .file_history(node.id, PATH)
        .await
        .unwrap()
        .versions;
    // La première écriture créait le fichier : rien à garder.
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0].reason, VersionReason::Write);
    assert_eq!(versions[0].channel, VersionChannel::Web);
    assert_eq!(versions[0].author, author());
    let latest = node
        .manager
        .file_version_read(node.id, PATH, versions[0].id)
        .await
        .unwrap();
    assert_eq!(latest.content, "motd=two\n");
    let oldest = node
        .manager
        .file_version_read(node.id, PATH, versions[1].id)
        .await
        .unwrap();
    assert_eq!(oldest.content, "motd=one\n");
}

#[tokio::test]
async fn a_restore_brings_a_version_back_and_can_be_undone() {
    let node = Node::new();
    node.write("motd=good\n").await;
    node.write("motd=broken\n").await;
    let good = node
        .manager
        .file_history(node.id, PATH)
        .await
        .unwrap()
        .versions[0]
        .id;

    node.manager
        .file_restore(node.id, PATH, good, author())
        .await
        .unwrap();
    assert_eq!(node.current().await, "motd=good\n");

    // La version écrasée par la restauration est gardée : la restauration s'annule.
    let versions = node
        .manager
        .file_history(node.id, PATH)
        .await
        .unwrap()
        .versions;
    assert_eq!(versions[0].reason, VersionReason::Restore);
    node.manager
        .file_restore(node.id, PATH, versions[0].id, author())
        .await
        .unwrap();
    assert_eq!(node.current().await, "motd=broken\n");
}

#[tokio::test]
async fn a_deleted_file_keeps_its_last_version() {
    let node = Node::new();
    node.write("motd=keep me\n").await;
    node.manager
        .file_delete_as(node.id, PATH, author())
        .await
        .unwrap();
    let versions = node
        .manager
        .file_history(node.id, PATH)
        .await
        .unwrap()
        .versions;
    assert_eq!(versions[0].reason, VersionReason::Delete);
    node.manager
        .file_restore(node.id, PATH, versions[0].id, author())
        .await
        .unwrap();
    assert_eq!(node.current().await, "motd=keep me\n");
}

#[tokio::test]
async fn the_history_stays_out_of_the_volumes() {
    let node = Node::new();
    node.write("a\n").await;
    node.write("b\n").await;
    let names: Vec<String> = node
        .manager
        .file_list(node.id, "/app/data")
        .await
        .unwrap()
        .entries
        .into_iter()
        .map(|entry| entry.name)
        .collect();
    assert_eq!(names, ["server.properties"]);
    assert!(
        node.data
            .path()
            .join(node.id.to_string())
            .join(".kuro/history/index.jsonl")
            .exists()
    );
}

#[tokio::test]
async fn a_version_of_another_file_or_an_unknown_one_is_not_found() {
    let node = Node::new();
    node.write("a\n").await;
    node.write("b\n").await;
    let version = node
        .manager
        .file_history(node.id, PATH)
        .await
        .unwrap()
        .versions[0]
        .id;
    assert_eq!(
        node.manager
            .file_version_read(node.id, "/app/data/other.txt", version)
            .await
            .unwrap_err(),
        CommandError::NotFound
    );
    assert_eq!(
        node.manager
            .file_restore(node.id, PATH, Uuid::now_v7(), None)
            .await
            .unwrap_err(),
        CommandError::NotFound
    );
    // Hors d'un volume : refusé comme une écriture.
    assert_eq!(
        node.manager
            .file_history(node.id, "/app/../../etc/passwd")
            .await
            .unwrap_err(),
        CommandError::InvalidPath
    );
}

#[tokio::test]
async fn deleted_files_are_listed_and_come_back_with_their_folder() {
    let node = Node::new();
    node.manager
        .file_mkdir(node.id, "/app/data/plugins")
        .await
        .unwrap();
    node.manager
        .file_write_as(
            node.id,
            "/app/data/plugins/config.yml",
            "enabled: true\n".to_owned(),
            None,
            author(),
        )
        .await
        .unwrap();
    node.manager
        .file_delete_as(node.id, "/app/data/plugins/config.yml", author())
        .await
        .unwrap();
    // Le dossier entier part ensuite.
    node.manager
        .file_delete_as(node.id, "/app/data/plugins", author())
        .await
        .unwrap();

    let deleted = node
        .manager
        .file_history_deleted(node.id, "/app/data/plugins")
        .await;
    // Le dossier n'existe plus : la liste se demande depuis un dossier existant, ou échoue.
    let files = match deleted {
        Ok(list) => list.files,
        Err(error) => panic!("{error}"),
    };
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "/app/data/plugins/config.yml");

    node.manager
        .file_restore(
            node.id,
            "/app/data/plugins/config.yml",
            files[0].version.id,
            author(),
        )
        .await
        .unwrap();
    let back = node
        .manager
        .file_read(node.id, "/app/data/plugins/config.yml")
        .await
        .unwrap();
    assert_eq!(back.content, "enabled: true\n");
}

#[tokio::test]
async fn a_switched_off_history_keeps_nothing_and_forgets_the_rest() {
    let node = Node::new();
    node.write("a\n").await;
    node.write("b\n").await;
    assert_eq!(
        node.manager
            .file_history(node.id, PATH)
            .await
            .unwrap()
            .versions
            .len(),
        1
    );

    node.manager
        .apply_history_policy(kuro_protocol::api::HistoryPolicy {
            days: 0,
            max_mb: 100,
        });
    // La purge tourne en tâche de fond.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while !node
        .manager
        .file_history(node.id, PATH)
        .await
        .unwrap()
        .versions
        .is_empty()
    {
        assert!(tokio::time::Instant::now() < deadline, "history not purged");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    node.write("c\n").await;
    assert!(
        node.manager
            .file_history(node.id, PATH)
            .await
            .unwrap()
            .versions
            .is_empty()
    );
}
