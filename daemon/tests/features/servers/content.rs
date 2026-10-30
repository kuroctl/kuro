//! Contenu des serveurs (ADR-0020) : dépôt vérifié par l'empreinte de la source (SHA-512 pour
//! Modrinth, SHA-1 pour CurseForge), activation, suppression, refus.

use std::collections::HashMap;
use std::time::Duration;

use kuro_daemon::docker::Docker;
use kuro_daemon::fs::ServerDir;
use kuro_daemon::server::{CommandError, Manager};
use kuro_protocol::api::server::ServerLimits;
use std::io::Write;

use kuro_protocol::api::{
    ContentExtract, ContentFile, ContentHash, ContentInstall, ContentRemove, ContentToggle,
    ExtractRoute,
};
use kuro_protocol::daemon::{ContentDone, ContentOutcome};
use sha1::Sha1;
use sha2::{Digest, Sha512};
use tempfile::TempDir;
use tokio::sync::mpsc;
use uuid::Uuid;

use super::https::{Route, TestServer, serve};
use super::support::{IMAGE, single, until_free};

const MOD: &[u8] = b"PK\x03\x04 fake mod jar";

struct Node {
    manager: Manager,
    content: mpsc::Receiver<ContentDone>,
    data: TempDir,
    id: Uuid,
    server: TestServer,
}

impl Node {
    /// Serveur déployé sans conteneur ; un serveur HTTPS local sert `/sodium.jar`.
    async fn new() -> Self {
        Self::serving(HashMap::new()).await
    }

    /// Comme [`Node::new`], avec d'autres fichiers servis.
    async fn serving(mut routes: HashMap<String, Route>) -> Self {
        routes.insert("/sodium.jar".to_owned(), Route::File(MOD.to_vec()));
        let server = serve(routes).await;
        let data = TempDir::new().unwrap();
        let docker = Docker::connect(IMAGE).unwrap();
        let (manager, outgoing) =
            Manager::with_downloads(docker, data.path().to_path_buf(), server.policy.clone());
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
            server,
        }
    }

    fn host(&self, relative: &str) -> std::path::PathBuf {
        self.data
            .path()
            .join(self.id.to_string())
            .join("volumes/app/data")
            .join(relative)
    }

    fn file(&self, sha512: &str) -> ContentFile {
        self.file_with(ContentHash::Sha512(sha512.to_owned()))
    }

    fn file_with(&self, hash: ContentHash) -> ContentFile {
        ContentFile {
            url: self.server.url("/sodium.jar"),
            hash,
            size: MOD.len() as u64,
            file_name: "sodium.jar".to_owned(),
        }
    }

    fn install(&self, files: Vec<ContentFile>) -> ContentInstall {
        ContentInstall {
            server_id: self.id,
            operation_id: Uuid::now_v7(),
            service: "app".to_owned(),
            dir: "/data/mods".to_owned(),
            files,
            root: None,
            archive: None,
            extract: None,
        }
    }

    async fn done(&mut self) -> ContentDone {
        tokio::time::timeout(Duration::from_secs(30), self.content.recv())
            .await
            .expect("no content.done")
            .expect("channel closed")
    }
}

fn sha512(bytes: &[u8]) -> String {
    Sha512::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn sha1(bytes: &[u8]) -> String {
    Sha1::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[tokio::test]
async fn a_mod_is_downloaded_checked_then_toggled_and_removed() {
    let mut node = Node::new().await;
    let install = node.install(vec![node.file(&sha512(MOD))]);
    let operation = install.operation_id;

    node.manager.content_install(install).await.unwrap();
    let done = node.done().await;

    assert_eq!(done.operation_id, operation);
    assert_eq!(done.outcome, ContentOutcome::Ok);
    assert_eq!(std::fs::read(node.host("mods/sodium.jar")).unwrap(), MOD);

    let toggle = |enabled| ContentToggle {
        server_id: node.id,
        service: "app".to_owned(),
        dir: "/data/mods".to_owned(),
        file_name: "sodium.jar".to_owned(),
        enabled,
    };
    node.manager.content_toggle(toggle(false)).await.unwrap();
    assert!(!node.host("mods/sodium.jar").exists());
    assert!(node.host("mods/sodium.jar.disabled").exists());
    // Déjà désactivé : sans effet.
    node.manager.content_toggle(toggle(false)).await.unwrap();
    node.manager.content_toggle(toggle(true)).await.unwrap();
    assert!(node.host("mods/sodium.jar").exists());

    node.manager.content_toggle(toggle(false)).await.unwrap();
    node.manager
        .content_remove(ContentRemove {
            server_id: node.id,
            service: "app".to_owned(),
            dir: "/data/mods".to_owned(),
            root: None,
            file_names: vec!["sodium.jar".to_owned(), "gone.jar".to_owned()],
            dirs: Vec::new(),
        })
        .await
        .unwrap();
    assert!(!node.host("mods/sodium.jar.disabled").exists());
}

#[tokio::test]
async fn a_wrong_checksum_leaves_no_file() {
    let mut node = Node::new().await;

    node.manager
        .content_install(node.install(vec![node.file(&"0".repeat(128))]))
        .await
        .unwrap();
    let done = node.done().await;

    assert!(
        matches!(&done.outcome, ContentOutcome::Failed { reason } if reason.contains("checksum")),
        "{:?}",
        done.outcome
    );
    let left: Vec<_> = std::fs::read_dir(node.host("mods"))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert!(left.is_empty(), "{left:?}");
}

#[tokio::test]
async fn a_curseforge_file_is_checked_by_its_sha1() {
    let mut node = Node::new().await;

    node.manager
        .content_install(node.install(vec![node.file_with(ContentHash::Sha1(sha1(MOD)))]))
        .await
        .unwrap();
    assert_eq!(node.done().await.outcome, ContentOutcome::Ok);
    assert_eq!(std::fs::read(node.host("mods/sodium.jar")).unwrap(), MOD);

    node.manager
        .content_install(node.install(vec![node.file_with(ContentHash::Sha1("0".repeat(40)))]))
        .await
        .unwrap();
    let done = node.done().await;
    assert!(
        matches!(&done.outcome, ContentOutcome::Failed { reason } if reason.contains("checksum")),
        "{:?}",
        done.outcome
    );
    // Une empreinte qui ne correspond pas à son algorithme est refusée avant tout téléchargement.
    assert!(matches!(
        node.manager
            .content_install(node.install(vec![node.file_with(ContentHash::Sha1(sha512(MOD)))]))
            .await,
        Err(CommandError::InvalidSpec(_))
    ));
}

#[tokio::test]
async fn paths_out_of_the_slot_are_refused() {
    let node = Node::new().await;
    let good = sha512(MOD);

    for dir in ["/etc", "/data", "/data/../etc"] {
        let mut install = node.install(vec![node.file(&good)]);
        install.dir = dir.to_owned();
        assert_eq!(
            node.manager.content_install(install).await,
            Err(CommandError::InvalidPath),
            "{dir}"
        );
    }
    for name in [
        "../escape.jar",
        "a/b.jar",
        ".hidden.jar",
        "x.jar.disabled",
        "",
    ] {
        let mut file = node.file(&good);
        file.file_name = name.to_owned();
        assert_eq!(
            node.manager.content_install(node.install(vec![file])).await,
            Err(CommandError::InvalidPath),
            "{name}"
        );
    }
    let mut plain = node.file(&good);
    plain.url = plain.url.replace("https://", "http://");
    assert!(matches!(
        node.manager
            .content_install(node.install(vec![plain]))
            .await,
        Err(CommandError::InvalidSpec(_))
    ));
}

#[tokio::test]
async fn quota_and_suspension_apply() {
    let node = Node::new().await;
    // Quota et suspension changent par les commandes du gestionnaire, qui garde la spécification.
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
            disk_bytes: 8,
        })
        .unwrap();

    assert_eq!(
        until_free(|| node
            .manager
            .content_install(node.install(vec![node.file(&sha512(MOD))])))
        .await,
        Err(CommandError::DiskQuotaExceeded)
    );

    until_free(|| async {
        node.manager.limits(ServerLimits {
            server_id: node.id,
            services: spec
                .services
                .iter()
                .map(|s| (s.name.clone(), s.limits))
                .collect(),
            disk_bytes: 0,
        })
    })
    .await
    .unwrap();
    until_free(|| async { node.manager.suspend(node.id, true) })
        .await
        .unwrap();
    assert_eq!(
        until_free(|| node
            .manager
            .content_install(node.install(vec![node.file(&sha512(MOD))])))
        .await,
        Err(CommandError::Suspended)
    );
}

/// Archive zip de `(chemin, contenu)`.
fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buffer = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(&mut buffer);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, content) in entries {
        writer.start_file(*name, options).unwrap();
        writer.write_all(content).unwrap();
    }
    writer.finish().unwrap();
    buffer.into_inner()
}

fn manifest(uuid: &str, kind: &str) -> Vec<u8> {
    format!(
        "\u{feff}{{\"format_version\": 2, \"header\": {{\"uuid\": \"{uuid}\", \"version\": [1, 0, 0]}}, \"modules\": [{{\"type\": \"{kind}\"}}]}}"
    )
    .into_bytes()
}

/// Installation d'add-on Bedrock : packs de comportement (`data`) et de ressources.
fn addon_install(node: &Node, archive: &[u8]) -> ContentInstall {
    ContentInstall {
        files: vec![ContentFile {
            url: node.server.url("/more-mobs.mcaddon"),
            hash: ContentHash::Sha512(sha512(archive)),
            size: archive.len() as u64,
            file_name: "more-mobs.mcaddon".to_owned(),
        }],
        dir: "/data/behavior_packs".to_owned(),
        extract: Some(ContentExtract {
            describe: "manifest.json".to_owned(),
            routes: vec![
                ExtractRoute {
                    value: "modules.0.type".to_owned(),
                    is: "data".to_owned(),
                    dir: "/data/behavior_packs".to_owned(),
                },
                ExtractRoute {
                    value: "modules.0.type".to_owned(),
                    is: "resources".to_owned(),
                    dir: "/data/resource_packs".to_owned(),
                },
            ],
        }),
        ..node.install(Vec::new())
    }
}

#[tokio::test]
async fn an_addon_is_split_into_packs_routed_by_their_manifest() {
    // Un `.mcaddon` porte deux `.mcpack` : comportement et ressources.
    let behavior = zip(&[
        ("manifest.json", &manifest("b-uuid", "data")),
        ("entities/mob.json", b"{}"),
    ]);
    let resources = zip(&[
        ("pack/manifest.json", &manifest("r-uuid", "resources")),
        ("pack/textures/mob.png", b"png"),
    ]);
    let archive = zip(&[
        ("More Mobs BP.mcpack", &behavior),
        ("More Mobs RP.mcpack", &resources),
    ]);
    let mut node = Node::serving(HashMap::from([(
        "/more-mobs.mcaddon".to_owned(),
        Route::File(archive.clone()),
    )]))
    .await;

    node.manager
        .content_install(addon_install(&node, &archive))
        .await
        .unwrap();
    let done = node.done().await;

    assert_eq!(done.outcome, ContentOutcome::Ok, "{:?}", done.outcome);
    let mut packages: Vec<_> = done
        .packages
        .iter()
        .map(|p| {
            (
                p.dir.as_str(),
                p.name.as_str(),
                p.description["header"]["uuid"].as_str().unwrap(),
            )
        })
        .collect();
    packages.sort_unstable();
    assert_eq!(
        packages,
        [
            ("/data/behavior_packs", "more-mobs-1", "b-uuid"),
            ("/data/resource_packs", "more-mobs-2", "r-uuid"),
        ]
    );
    assert!(
        node.host("behavior_packs/more-mobs-1/entities/mob.json")
            .exists()
    );
    assert!(
        node.host("resource_packs/more-mobs-2/textures/mob.png")
            .exists()
    );
    // Ni l'archive ni le dossier temporaire ne restent.
    let left: Vec<_> = std::fs::read_dir(node.host("behavior_packs"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(left, ["more-mobs-1"]);

    // Supprimer l'add-on retire ses dossiers.
    node.manager
        .content_remove(ContentRemove {
            server_id: node.id,
            service: "app".to_owned(),
            dir: "/data/behavior_packs".to_owned(),
            root: None,
            file_names: Vec::new(),
            dirs: vec![
                "/data/behavior_packs/more-mobs-1".to_owned(),
                "/data/resource_packs/more-mobs-2".to_owned(),
            ],
        })
        .await
        .unwrap();
    assert!(!node.host("behavior_packs/more-mobs-1").exists());
    assert!(!node.host("resource_packs/more-mobs-2").exists());
}

#[tokio::test]
async fn a_pack_without_a_route_fails_the_install_and_leaves_nothing() {
    let archive = zip(&[("manifest.json", &manifest("s-uuid", "skin_pack"))]);
    let mut node = Node::serving(HashMap::from([(
        "/more-mobs.mcaddon".to_owned(),
        Route::File(archive.clone()),
    )]))
    .await;

    node.manager
        .content_install(addon_install(&node, &archive))
        .await
        .unwrap();
    let done = node.done().await;

    assert!(
        matches!(&done.outcome, ContentOutcome::Failed { reason } if reason.contains("no route")),
        "{:?}",
        done.outcome
    );
    assert!(done.packages.is_empty());
    assert_eq!(
        std::fs::read_dir(node.host("behavior_packs"))
            .unwrap()
            .count(),
        0
    );
}

#[tokio::test]
async fn an_extraction_route_outside_the_volumes_is_refused() {
    let node = Node::new().await;
    let mut install = addon_install(&node, b"x");
    install.extract.as_mut().unwrap().routes[0].dir = "/etc/packs".to_owned();

    assert!(matches!(
        node.manager.content_install(install).await,
        Err(CommandError::InvalidPath)
    ));
}
