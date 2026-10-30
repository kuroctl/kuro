//! Envoi de fichiers par morceaux et modpacks (ADR-0020).

use std::collections::HashMap;
use std::io::Write;
use std::time::Duration;

use base64::Engine;
use kuro_daemon::docker::Docker;
use kuro_daemon::fs::ServerDir;
use kuro_daemon::server::{CommandError, Manager};
use kuro_protocol::api::server::ServerLimits;
use kuro_protocol::api::{
    ContentFile, ContentHash, ContentInstall, ContentRemove, FileUpload, FileUploadChunk,
};
use kuro_protocol::daemon::{ContentDone, ContentOutcome};
use sha2::{Digest, Sha256, Sha512};
use tempfile::TempDir;
use tokio::sync::mpsc;
use uuid::Uuid;

use super::https::{Route, TestServer, serve};
use super::support::{IMAGE, single, until_free};

struct Node {
    manager: Manager,
    content: mpsc::Receiver<ContentDone>,
    data: TempDir,
    id: Uuid,
}

impl Node {
    fn new(server: Option<&TestServer>) -> Self {
        let data = TempDir::new().unwrap();
        let docker = Docker::connect(IMAGE).unwrap();
        let (manager, outgoing) = match server {
            Some(server) => {
                Manager::with_downloads(docker, data.path().to_path_buf(), server.policy.clone())
            }
            None => Manager::new(docker, data.path().to_path_buf()),
        };
        let id = Uuid::now_v7();
        let dir = ServerDir::new(data.path(), id);
        let spec = single(id);
        dir.save_spec(&spec).unwrap();
        dir.ensure_volumes(&spec).unwrap();
        Self {
            manager,
            content: outgoing.content,
            data,
            id,
        }
    }

    fn host(&self, relative: &str) -> std::path::PathBuf {
        self.data
            .path()
            .join(self.id.to_string())
            .join("volumes/app/data")
            .join(relative)
    }

    async fn start(&self, upload_id: Uuid, path: &str, size: u64) -> Result<(), CommandError> {
        self.manager
            .file_upload(FileUpload {
                server_id: self.id,
                upload_id,
                path: path.to_owned(),
                size,
            })
            .await
    }

    async fn chunk(
        &self,
        upload_id: Uuid,
        offset: u64,
        data: &[u8],
        last: Option<&str>,
    ) -> Result<(), CommandError> {
        self.manager
            .file_upload_chunk(FileUploadChunk {
                server_id: self.id,
                upload_id,
                offset,
                data: base64::engine::general_purpose::STANDARD.encode(data),
                eof: last.is_some(),
                sha256: last.map(str::to_owned),
            })
            .await
    }
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn sha512(bytes: &[u8]) -> String {
    Sha512::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[tokio::test]
async fn an_upload_lands_only_once_complete_and_verified() {
    let node = Node::new(None);
    let upload = Uuid::now_v7();
    let body = b"first half second half";

    node.start(upload, "/app/data/mods/mine.jar", body.len() as u64)
        .await
        .unwrap();
    node.chunk(upload, 0, &body[..11], None).await.unwrap();
    assert!(
        !node.host("mods/mine.jar").exists(),
        "visible before the end"
    );
    node.chunk(upload, 11, &body[11..], Some(&sha256(body)))
        .await
        .unwrap();

    assert_eq!(std::fs::read(node.host("mods/mine.jar")).unwrap(), body);
}

#[tokio::test]
async fn a_bad_upload_leaves_nothing_behind() {
    let node = Node::new(None);
    let body = b"abcdef";

    // Empreinte fausse.
    let upload = Uuid::now_v7();
    node.start(upload, "/app/data/x.jar", 6).await.unwrap();
    assert_eq!(
        node.chunk(upload, 0, body, Some(&"0".repeat(64))).await,
        Err(CommandError::Conflict)
    );
    // Morceau hors d'ordre : l'envoi est abandonné.
    let upload = Uuid::now_v7();
    node.start(upload, "/app/data/y.jar", 6).await.unwrap();
    assert_eq!(
        node.chunk(upload, 3, body, None).await,
        Err(CommandError::Conflict)
    );
    assert_eq!(
        node.chunk(upload, 0, body, None).await,
        Err(CommandError::NotFound)
    );
    // Plus que la taille annoncée.
    let upload = Uuid::now_v7();
    node.start(upload, "/app/data/z.jar", 2).await.unwrap();
    assert_eq!(
        node.chunk(upload, 0, body, None).await,
        Err(CommandError::TooLarge)
    );

    let left: Vec<_> = std::fs::read_dir(node.host(""))
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name())
        .collect();
    assert!(left.is_empty(), "{left:?}");
}

#[tokio::test]
async fn an_upload_stays_under_a_volume_and_within_the_quota() {
    let node = Node::new(None);
    for path in [
        "/app/data",
        "/app/elsewhere/x.jar",
        "/app/data/../x.jar",
        "/app/data/.kuro-x",
    ] {
        assert_eq!(
            node.start(Uuid::now_v7(), path, 1).await,
            Err(if path == "/app/elsewhere/x.jar" {
                CommandError::NotFound
            } else {
                CommandError::InvalidPath
            }),
            "{path}"
        );
    }
    assert_eq!(
        node.start(Uuid::now_v7(), "/app/data/huge.jar", 1 << 40)
            .await,
        Err(CommandError::TooLarge)
    );

    // Le quota change par la commande du gestionnaire, qui garde la spécification.
    let spec = ServerDir::new(node.data.path(), node.id)
        .load_spec()
        .unwrap()
        .unwrap();
    node.manager
        .limits(ServerLimits {
            server_id: node.id,
            services: spec
                .services
                .iter()
                .map(|s| (s.name.clone(), s.limits))
                .collect(),
            disk_bytes: 4,
        })
        .unwrap();
    assert_eq!(
        until_free(|| node.start(Uuid::now_v7(), "/app/data/x.jar", 10)).await,
        Err(CommandError::DiskQuotaExceeded)
    );
}

/// Archive `.mrpack` : index, `overrides/` et `server-overrides/`.
fn mrpack() -> Vec<u8> {
    let mut buffer = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(&mut buffer);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, content) in [
        ("modrinth.index.json", b"{}".as_slice()),
        ("overrides/config/pack.toml", b"a = 1".as_slice()),
        (
            "server-overrides/server.properties",
            b"motd=pack".as_slice(),
        ),
        ("client-overrides/options.txt", b"client only".as_slice()),
    ] {
        writer.start_file(name, options).unwrap();
        writer.write_all(content).unwrap();
    }
    writer.finish().unwrap();
    buffer.into_inner()
}

#[tokio::test]
async fn a_modpack_drops_its_files_and_overrides_into_the_game_dir() {
    let pack = mrpack();
    let jar = b"PK fake mod".to_vec();
    let server = serve(HashMap::from([
        ("/pack.mrpack".to_owned(), Route::File(pack.clone())),
        ("/sodium.jar".to_owned(), Route::File(jar.clone())),
    ]))
    .await;
    let mut node = Node::new(Some(&server));
    let file = |path: &str, name: &str, bytes: &[u8]| ContentFile {
        url: server.url(path),
        hash: ContentHash::Sha512(sha512(bytes)),
        size: bytes.len() as u64,
        file_name: name.to_owned(),
    };

    node.manager
        .content_install(ContentInstall {
            server_id: node.id,
            operation_id: Uuid::now_v7(),
            service: "app".to_owned(),
            dir: "/data/mods".to_owned(),
            files: vec![file("/sodium.jar", "mods/sodium.jar", &jar)],
            root: Some("/data".to_owned()),
            archive: Some(file("/pack.mrpack", "pack.mrpack", &pack)),
            extract: None,
        })
        .await
        .unwrap();
    let done = tokio::time::timeout(Duration::from_secs(30), node.content.recv())
        .await
        .unwrap()
        .unwrap();

    assert_eq!(done.outcome, ContentOutcome::Ok, "{:?}", done.outcome);
    assert_eq!(std::fs::read(node.host("mods/sodium.jar")).unwrap(), jar);
    assert_eq!(
        std::fs::read_to_string(node.host("config/pack.toml")).unwrap(),
        "a = 1"
    );
    assert_eq!(
        std::fs::read_to_string(node.host("server.properties")).unwrap(),
        "motd=pack"
    );
    assert!(!node.host("options.txt").exists());
    assert!(
        std::fs::read_dir(node.host(""))
            .unwrap()
            .filter_map(Result::ok)
            .all(|e| !e.file_name().to_string_lossy().starts_with(".kuro")),
        "archive left behind"
    );

    node.manager
        .content_remove(ContentRemove {
            server_id: node.id,
            service: "app".to_owned(),
            dir: "/data/mods".to_owned(),
            root: Some("/data".to_owned()),
            file_names: vec!["mods/sodium.jar".to_owned()],
            dirs: Vec::new(),
        })
        .await
        .unwrap();
    assert!(!node.host("mods/sodium.jar").exists());
}

#[tokio::test]
async fn modpack_paths_cannot_leave_the_game_dir() {
    let mut node = Node::new(None);
    for name in [
        "../escape.jar",
        "/etc/x",
        ".kuro-pack-x.mrpack",
        "mods/x.jar.disabled",
    ] {
        let result = node
            .manager
            .content_install(ContentInstall {
                server_id: node.id,
                operation_id: Uuid::now_v7(),
                service: "app".to_owned(),
                dir: "/data/mods".to_owned(),
                files: vec![ContentFile {
                    url: "https://cdn.modrinth.com/x.jar".to_owned(),
                    hash: ContentHash::Sha512("a".repeat(128)),
                    size: 1,
                    file_name: name.to_owned(),
                }],
                root: Some("/data".to_owned()),
                archive: None,
                extract: None,
            })
            .await;
        assert_eq!(result, Err(CommandError::InvalidPath), "{name}");
    }
    assert!(node.content.try_recv().is_err());
}
