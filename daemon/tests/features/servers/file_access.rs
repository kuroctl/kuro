//! Fichiers des volumes (ADR-0015) : lecture, écriture atomique, conflits, et confinement. Sous
//! Linux, le chemin `openat2` ; ailleurs, le repli de développement. Les cas de liens
//! symboliques ne tournent que sous Unix.

use kuro_daemon::docker::Docker;
use kuro_daemon::fs::ServerDir;
use kuro_daemon::server::{CommandError, Manager};
use kuro_protocol::daemon::FileKind;
use tempfile::TempDir;
use uuid::Uuid;

use super::support::{IMAGE, single};

struct Node {
    manager: Manager,
    data: TempDir,
    id: Uuid,
}

impl Node {
    /// Serveur déployé (spécification et volumes), sans conteneur : seuls les fichiers comptent.
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

    /// Chemin sur l'hôte, sous le volume `data` du service `app`.
    fn host(&self, relative: &str) -> std::path::PathBuf {
        self.data
            .path()
            .join(self.id.to_string())
            .join("volumes/app/data")
            .join(relative)
    }

    fn put(&self, relative: &str, content: &str) {
        let path = self.host(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    async fn names(&self, path: &str) -> Vec<(String, FileKind)> {
        self.manager
            .file_list(self.id, path)
            .await
            .unwrap()
            .entries
            .into_iter()
            .map(|e| (e.name, e.kind))
            .collect()
    }
}

#[tokio::test]
async fn the_root_shows_services_then_their_volumes() {
    let node = Node::new();

    assert_eq!(
        node.names("/").await,
        [("app".to_owned(), FileKind::Directory)]
    );
    assert_eq!(
        node.names("/app").await,
        [("data".to_owned(), FileKind::Directory)]
    );
}

#[tokio::test]
async fn a_volume_lists_directories_first_without_temporary_files() {
    let node = Node::new();
    node.put("server.properties", "motd=x\n");
    node.put("world/level.dat", "x");
    node.put(".kuro-tmp-0192", "half written");

    assert_eq!(
        node.names("/app/data").await,
        [
            ("world".to_owned(), FileKind::Directory),
            ("server.properties".to_owned(), FileKind::File),
        ]
    );
}

#[tokio::test]
async fn a_text_file_is_read_with_its_fingerprint() {
    let node = Node::new();
    node.put("server.properties", "motd=Kuro\n");

    let file = node
        .manager
        .file_read(node.id, "/app/data/server.properties")
        .await
        .unwrap();

    assert_eq!(file.content, "motd=Kuro\n");
    assert_eq!(file.size, 10);
    assert_eq!(file.sha256, kuro_runtime::tls::sha256_hex(b"motd=Kuro\n"));
    assert!(file.modified_ms > 0);
}

#[tokio::test]
async fn binary_or_oversized_files_are_not_opened() {
    let node = Node::new();
    std::fs::write(node.host("level.dat"), [0x1f, 0x8b, 0, 1]).unwrap();
    node.put("huge.log", &"a".repeat(512 * 1024 + 1));

    assert_eq!(
        node.manager.file_read(node.id, "/app/data/level.dat").await,
        Err(CommandError::NotText)
    );
    assert_eq!(
        node.manager.file_read(node.id, "/app/data/huge.log").await,
        Err(CommandError::TooLarge)
    );
}

#[tokio::test]
async fn a_write_replaces_the_file_only_if_unchanged() {
    let node = Node::new();
    node.put("server.properties", "motd=old\n");
    let read = node
        .manager
        .file_read(node.id, "/app/data/server.properties")
        .await
        .unwrap();
    // Le serveur réécrit le fichier entre la lecture et l'enregistrement.
    node.put("server.properties", "motd=game\n");

    let stale = node
        .manager
        .file_write(
            node.id,
            "/app/data/server.properties",
            "motd=mine\n".to_owned(),
            Some(read.sha256),
        )
        .await;
    assert_eq!(stale, Err(CommandError::Conflict));

    let fresh = node
        .manager
        .file_read(node.id, "/app/data/server.properties")
        .await
        .unwrap();
    node.manager
        .file_write(
            node.id,
            "/app/data/server.properties",
            "motd=mine\n".to_owned(),
            Some(fresh.sha256),
        )
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(node.host("server.properties")).unwrap(),
        "motd=mine\n"
    );
}

#[tokio::test]
async fn a_new_file_and_directory_are_created() {
    let node = Node::new();

    node.manager
        .file_mkdir(node.id, "/app/data/plugins")
        .await
        .unwrap();
    node.manager
        .file_write(
            node.id,
            "/app/data/plugins/config.yml",
            "enabled: true\n".to_owned(),
            None,
        )
        .await
        .unwrap();

    assert_eq!(
        std::fs::read_to_string(node.host("plugins/config.yml")).unwrap(),
        "enabled: true\n"
    );
    assert_eq!(
        node.manager.file_mkdir(node.id, "/app/data/plugins").await,
        Err(CommandError::AlreadyExists)
    );
}

#[tokio::test]
async fn a_rename_never_overwrites() {
    let node = Node::new();
    node.put("a.txt", "a");
    node.put("b.txt", "b");
    node.put("dir/inner.txt", "x");

    assert_eq!(
        node.manager
            .file_rename(node.id, "/app/data/a.txt", "/app/data/b.txt")
            .await,
        Err(CommandError::AlreadyExists)
    );
    assert_eq!(
        node.manager
            .file_rename(node.id, "/app/data/dir", "/app/data/dir/sub")
            .await,
        Err(CommandError::InvalidPath)
    );
    node.manager
        .file_rename(node.id, "/app/data/a.txt", "/app/data/dir/c.txt")
        .await
        .unwrap();

    assert!(!node.host("a.txt").exists());
    assert_eq!(
        std::fs::read_to_string(node.host("dir/c.txt")).unwrap(),
        "a"
    );
}

#[tokio::test]
async fn a_directory_is_deleted_with_its_content() {
    let node = Node::new();
    node.put("world/region/r.0.0.mca", "x");
    node.put("server.properties", "x");

    node.manager
        .file_delete(node.id, "/app/data/world")
        .await
        .unwrap();
    node.manager
        .file_delete(node.id, "/app/data/server.properties")
        .await
        .unwrap();

    assert!(!node.host("world").exists());
    assert!(!node.host("server.properties").exists());
    assert_eq!(
        node.manager.file_delete(node.id, "/app/data/world").await,
        Err(CommandError::NotFound)
    );
}

#[tokio::test]
async fn nothing_outside_the_volumes_is_reachable() {
    let node = Node::new();

    for path in [
        "/app/data/../../../.kuro/spec.json",
        "/app/../../.kuro/spec.json",
        "/app/data/./x",
        "/app/data\\..\\..\\x",
    ] {
        assert_eq!(
            node.manager.file_read(node.id, path).await,
            Err(CommandError::InvalidPath),
            "{path}"
        );
    }
    // Service ou volume inconnu de la spécification.
    assert_eq!(
        node.manager.file_read(node.id, "/db/data/x").await,
        Err(CommandError::NotFound)
    );
    assert_eq!(
        node.manager.file_list(node.id, "/app/secrets").await,
        Err(CommandError::NotFound)
    );
}

#[tokio::test]
async fn mount_points_cannot_be_changed() {
    let node = Node::new();

    assert_eq!(
        node.manager.file_delete(node.id, "/app/data").await,
        Err(CommandError::InvalidPath)
    );
    assert_eq!(
        node.manager.file_mkdir(node.id, "/app/other").await,
        Err(CommandError::InvalidPath)
    );
    assert_eq!(
        node.manager
            .file_write(node.id, "/app/data", String::new(), None)
            .await,
        Err(CommandError::InvalidPath)
    );
}

#[tokio::test]
async fn an_unknown_server_has_no_files() {
    let node = Node::new();

    assert_eq!(
        node.manager.file_list(Uuid::now_v7(), "/").await,
        Err(CommandError::UnknownServer)
    );
}

#[cfg(unix)]
mod symlinks {
    //! Un conteneur peut créer des liens dans son volume : aucun n'est jamais suivi.

    use std::os::unix::fs::symlink;

    use super::*;

    #[tokio::test]
    async fn a_link_to_the_outside_is_listed_but_never_followed() {
        let node = Node::new();
        let secret = node
            .data
            .path()
            .join(node.id.to_string())
            .join(".kuro/spec.json");
        symlink(&secret, node.host("spec")).unwrap();
        symlink("/etc", node.host("etc")).unwrap();

        assert!(
            node.names("/app/data")
                .await
                .contains(&("spec".to_owned(), FileKind::Symlink))
        );
        assert_eq!(
            node.manager.file_read(node.id, "/app/data/spec").await,
            Err(CommandError::InvalidPath)
        );
        assert_eq!(
            node.manager.file_list(node.id, "/app/data/etc").await,
            Err(CommandError::InvalidPath)
        );
        assert_eq!(
            node.manager
                .file_read(node.id, "/app/data/etc/passwd")
                .await,
            Err(CommandError::InvalidPath)
        );
        assert_eq!(
            node.manager
                .file_write(node.id, "/app/data/spec", "pwned".to_owned(), None)
                .await,
            Err(CommandError::InvalidPath)
        );
        assert_ne!(std::fs::read_to_string(&secret).unwrap(), "pwned");
    }

    #[tokio::test]
    async fn even_an_inner_link_is_not_followed() {
        let node = Node::new();
        node.put("real/file.txt", "x");
        symlink(node.host("real"), node.host("alias")).unwrap();

        assert_eq!(
            node.manager
                .file_read(node.id, "/app/data/alias/file.txt")
                .await,
            Err(CommandError::InvalidPath)
        );
    }

    #[tokio::test]
    async fn deleting_a_link_keeps_its_target() {
        let node = Node::new();
        let outside = node.data.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("keep.txt"), "x").unwrap();
        symlink(&outside, node.host("link")).unwrap();
        node.put("dir/x", "x");
        symlink(&outside, node.host("dir/link")).unwrap();

        node.manager
            .file_delete(node.id, "/app/data/link")
            .await
            .unwrap();
        node.manager
            .file_delete(node.id, "/app/data/dir")
            .await
            .unwrap();

        assert!(outside.join("keep.txt").exists());
        assert!(std::fs::symlink_metadata(node.host("link")).is_err());
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn created_files_belong_to_the_service_user_and_edits_keep_the_owner() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let node = Node::new();
    node.put("existing.cfg", "x");
    let me = std::fs::metadata(node.host("existing.cfg")).unwrap().uid();
    if me != 0 {
        // Changer de propriétaire exige root (comme le daemon en production).
        return;
    }
    std::fs::set_permissions(
        node.host("existing.cfg"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();

    node.manager
        .file_write(node.id, "/app/data/new.cfg", "x".to_owned(), None)
        .await
        .unwrap();
    node.manager
        .file_mkdir(node.id, "/app/data/plugins")
        .await
        .unwrap();
    node.manager
        .file_write(node.id, "/app/data/existing.cfg", "y".to_owned(), None)
        .await
        .unwrap();

    // `single()` : service `app` en 1000:1000.
    for created in ["new.cfg", "plugins"] {
        let meta = std::fs::metadata(node.host(created)).unwrap();
        assert_eq!((meta.uid(), meta.gid()), (1000, 1000), "{created}");
    }
    let edited = std::fs::metadata(node.host("existing.cfg")).unwrap();
    assert_eq!((edited.uid(), edited.mode() & 0o777), (0, 0o600));
}
