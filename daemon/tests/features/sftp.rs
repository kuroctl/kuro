//! SFTP de bout en bout côté nœud (ADR-0047) : un vrai client SSH/SFTP, le processus SFTP, le
//! pont du daemon et sa file d'audit ; l'API est simulée sur la file des requêtes du lien.
//! Linux seulement, comme le processus SFTP (`openat2`).

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kuro_daemon::docker::Docker;
use kuro_daemon::fs::ServerDir;
use kuro_daemon::server::Manager;
use kuro_daemon::sftp::SftpConfig;
use kuro_daemon::sftp::spool::Spool;
use kuro_daemon::sftp::{bridge, hostkey, worker};
use kuro_protocol::api::sftp::{SftpAuditAck, SftpDenial, SftpGrant, SftpLeaseResult, SftpLeases};
use kuro_protocol::daemon::sftp::{
    CloseReason, SftpCredentialRef, SftpEvent, SftpEventKind, SftpPrincipal,
};
use kuro_protocol::{ApiMessage, DaemonMessage};
use russh::keys::{PrivateKey, PrivateKeyWithHashAlg};
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::FileAttributes;
use tempfile::TempDir;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::servers::support::{IMAGE, single};

/// Comptes connus de l'API simulée : `serveur.compte` → serveur accordé.
const USER_A: &str = "servera.alice";
const USER_B: &str = "serverb.alice";
const ADMIN: Uuid = Uuid::from_u128(0xa11ce);

#[derive(Default)]
struct FakeApi {
    auths: AtomicUsize,
    /// Bail accordé au renouvellement.
    renew: AtomicBool,
    write: AtomicBool,
    events: Mutex<Vec<SftpEvent>>,
}

impl FakeApi {
    fn events(&self) -> Vec<SftpEvent> {
        self.events.lock().unwrap().clone()
    }

    async fn wait_for(&self, what: impl Fn(&SftpEvent) -> bool) -> SftpEvent {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(event) = self.events().into_iter().find(|e| what(e)) {
                return event;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "event not received: {:?}",
                self.events()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

struct Node {
    _data: TempDir,
    manager: Manager,
    root: std::path::PathBuf,
    a: Uuid,
    b: Uuid,
    addr: std::net::SocketAddr,
    api: Arc<FakeApi>,
    client_key: Arc<PrivateKey>,
    shutdown: CancellationToken,
}

impl Drop for Node {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

fn deploy(data: &Path, id: Uuid) {
    let dir = ServerDir::new(data, id);
    let spec = single(id);
    dir.save_spec(&spec).unwrap();
    dir.ensure_volumes(&spec).unwrap();
}

impl Node {
    async fn start() -> Self {
        let data = TempDir::new().unwrap();
        let root = data.path().join("servers");
        std::fs::create_dir_all(&root).unwrap();
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        deploy(&root, a);
        deploy(&root, b);
        let docker = Docker::connect(IMAGE).unwrap();
        let (manager, _) = Manager::new(docker, root.clone());
        let kept = manager.clone();

        let api = Arc::new(FakeApi {
            renew: AtomicBool::new(true),
            ..FakeApi::default()
        });
        let (requests, mut queue) = kuro_daemon::link::channel();
        tokio::spawn({
            let api = Arc::clone(&api);
            async move {
                while let Some((message, reply)) = queue.next().await {
                    let answer = match message {
                        DaemonMessage::SftpAuth(auth) => {
                            api.auths.fetch_add(1, Ordering::SeqCst);
                            let server = match auth.username.as_str() {
                                USER_A => Some(a),
                                USER_B => Some(b),
                                _ => None,
                            };
                            ApiMessage::SftpGrant(match server {
                                Some(server_id) => SftpGrant::Granted {
                                    server_id,
                                    principal: SftpPrincipal::Admin(ADMIN),
                                    credential: SftpCredentialRef::Key(Uuid::nil()),
                                    write: api.write.load(Ordering::SeqCst),
                                    lease_secs: 1,
                                },
                                None => SftpGrant::Denied {
                                    reason: SftpDenial::UnknownAccount,
                                },
                            })
                        }
                        DaemonMessage::SftpLease(lease) => ApiMessage::SftpLeases(SftpLeases {
                            results: lease
                                .sessions
                                .iter()
                                .map(|session| SftpLeaseResult {
                                    session_id: session.session_id,
                                    granted: api.renew.load(Ordering::SeqCst),
                                    write: api.write.load(Ordering::SeqCst),
                                })
                                .collect(),
                        }),
                        DaemonMessage::SftpAudit(audit) => {
                            let recorded = u32::try_from(audit.events.len()).unwrap();
                            api.events.lock().unwrap().extend(audit.events);
                            ApiMessage::SftpAuditAck(SftpAuditAck { recorded })
                        }
                        other => panic!("unexpected request {other:?}"),
                    };
                    let _ = reply.send(Ok(answer));
                }
            }
        });

        let shutdown = CancellationToken::new();
        let config = SftpConfig {
            socket: data.path().join("sftp.sock"),
            spool: data.path().join("spool"),
            worker_user: None,
            ..SftpConfig::default()
        };
        let spool = Arc::new(Spool::open(&config.spool).unwrap());
        tokio::spawn(bridge::forward_audit(
            Arc::clone(&spool),
            requests.clone(),
            shutdown.clone(),
        ));
        tokio::spawn({
            let config = config.clone();
            let shutdown = shutdown.clone();
            async move {
                bridge::serve(&config, Some(manager), requests, spool, shutdown)
                    .await
                    .unwrap();
            }
        });
        // Le socket existe une fois le pont à l'écoute.
        while !config.socket.exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        let host_key = hostkey::ensure(&data.path().join("host_ed25519")).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(worker::serve(
            listener,
            Arc::new(worker::ssh_config(host_key)),
            config.socket.clone(),
            root.clone(),
            Duration::from_millis(300),
            shutdown.clone(),
        ));
        let client_key = Arc::new(hostkey::ensure(&data.path().join("client_ed25519")).unwrap());
        Self {
            _data: data,
            manager: kept,
            root,
            a,
            b,
            addr,
            api,
            client_key,
            shutdown,
        }
    }

    /// Chemin sur l'hôte, dans le volume du serveur.
    fn host(&self, server: Uuid, relative: &str) -> std::path::PathBuf {
        self.root
            .join(server.to_string())
            .join("volumes/app/data")
            .join(relative)
    }

    async fn ssh(&self, user: &str) -> Result<russh::client::Handle<Client>, String> {
        let config = Arc::new(russh::client::Config::default());
        let mut handle = russh::client::connect(config, self.addr, Client)
            .await
            .map_err(|e| e.to_string())?;
        let auth = handle
            .authenticate_publickey(
                user,
                PrivateKeyWithHashAlg::new(Arc::clone(&self.client_key), None),
            )
            .await
            .map_err(|e| e.to_string())?;
        if !auth.success() {
            return Err("authentication refused".to_owned());
        }
        Ok(handle)
    }

    async fn sftp(&self, user: &str) -> (russh::client::Handle<Client>, SftpSession) {
        let handle = self.ssh(user).await.unwrap();
        let channel = handle.channel_open_session().await.unwrap();
        channel.request_subsystem(true, "sftp").await.unwrap();
        let sftp = SftpSession::new(channel.into_stream()).await.unwrap();
        (handle, sftp)
    }
}

struct Client;

impl russh::client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

#[tokio::test]
async fn a_session_reads_its_server_and_nothing_else() {
    let node = Node::start().await;
    std::fs::write(node.host(node.a, "hello.txt"), "hello from a").unwrap();
    std::fs::create_dir_all(node.host(node.a, "world")).unwrap();
    std::fs::write(node.host(node.b, "secret.txt"), "secret of b").unwrap();

    let (_handle, sftp) = node.sftp(USER_A).await;
    // Un seul service, un seul volume : il est la racine.
    let mut names: Vec<String> = sftp
        .read_dir("/")
        .await
        .unwrap()
        .map(|entry| entry.file_name())
        .collect();
    names.sort();
    assert!(names.contains(&"hello.txt".to_owned()), "{names:?}");
    assert!(names.contains(&"world".to_owned()), "{names:?}");
    assert_eq!(sftp.read("/hello.txt").await.unwrap(), b"hello from a");
    assert_eq!(sftp.canonicalize("/world/../..").await.unwrap(), "/");

    // `..` reste dans la racine : le fichier de B n'est jamais atteint.
    let escape = format!("/../../{}/volumes/app/data/secret.txt", node.b);
    assert!(sftp.read(escape.as_str()).await.is_err());
    assert!(sftp.read("/../secret.txt").await.is_err());
}

#[tokio::test]
async fn symbolic_links_are_listed_but_never_followed() {
    let node = Node::start().await;
    std::fs::write(node.host(node.b, "secret.txt"), "secret of b").unwrap();
    // Lien posé par le « conteneur » de A vers un fichier de B, et vers un dossier hors volume.
    std::os::unix::fs::symlink(node.host(node.b, "secret.txt"), node.host(node.a, "link")).unwrap();
    std::os::unix::fs::symlink("/etc", node.host(node.a, "etc")).unwrap();

    let (_handle, sftp) = node.sftp(USER_A).await;
    assert!(sftp.read("/link").await.is_err());
    assert!(sftp.read("/etc/passwd").await.is_err());
    assert!(sftp.read_dir("/etc").await.is_err());
    let link = sftp.symlink_metadata("/link").await.unwrap();
    assert!(link.file_type().is_symlink());
    // `stat` ne suit pas non plus le lien.
    assert!(
        sftp.metadata("/link")
            .await
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(sftp.read_link("/link").await.is_err());
}

#[tokio::test]
async fn writes_are_refused_without_the_permission_and_reported_once() {
    let node = Node::start().await;
    let (_handle, sftp) = node.sftp(USER_A).await;
    assert!(sftp.create("/new.txt").await.is_err());
    assert!(sftp.create_dir("/dir").await.is_err());
    assert!(sftp.remove_file("/hello.txt").await.is_err());
    assert!(!node.host(node.a, "new.txt").exists());

    let denied = node
        .api
        .wait_for(|e| matches!(e.event, SftpEventKind::AccessDenied { .. }))
        .await;
    assert_eq!(denied.server_id, node.a);
    assert_eq!(
        denied.event,
        SftpEventKind::AccessDenied {
            permission: "servers.files.write".to_owned()
        }
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    let reports = node
        .api
        .events()
        .into_iter()
        .filter(|e| matches!(e.event, SftpEventKind::AccessDenied { .. }))
        .count();
    assert_eq!(reports, 1);
}

#[tokio::test]
async fn a_refused_lease_closes_the_session_and_its_summary_is_recorded() {
    let node = Node::start().await;
    std::fs::write(node.host(node.a, "hello.txt"), "hello").unwrap();
    let (_handle, sftp) = node.sftp(USER_A).await;
    assert_eq!(sftp.read("/hello.txt").await.unwrap(), b"hello");

    node.api.renew.store(false, Ordering::SeqCst);
    let closed = node
        .api
        .wait_for(|e| matches!(e.event, SftpEventKind::SessionClosed(_)))
        .await;
    let SftpEventKind::SessionClosed(summary) = closed.event else {
        unreachable!()
    };
    assert_eq!(summary.reason, CloseReason::LeaseDenied);
    assert_eq!(summary.files_read, 1);
    assert_eq!(summary.bytes_read, 5);
    assert_eq!(closed.principal, SftpPrincipal::Admin(ADMIN));
    // La session est fermée : plus rien ne passe.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(sftp.read("/hello.txt").await.is_err());
}

#[tokio::test]
async fn badly_formed_or_unknown_users_are_refused() {
    let node = Node::start().await;
    assert!(node.ssh("root").await.is_err());
    assert_eq!(
        node.api.auths.load(Ordering::SeqCst),
        0,
        "root reached the api"
    );

    assert!(node.ssh("nobody.alice").await.is_err());
    assert_eq!(node.api.auths.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_server_unknown_to_the_node_is_refused_even_if_granted() {
    let node = Node::start().await;
    // L'API accorde B, mais le nœud n'a plus sa spécification.
    std::fs::remove_dir_all(node.root.join(node.b.to_string())).unwrap();
    assert!(node.ssh(USER_B).await.is_err());
    assert!(node.ssh(USER_A).await.is_ok());
}

/// Réponse du serveur à une requête de canal : `true` si refusée, sans aucune donnée.
async fn refused(mut channel: russh::Channel<russh::client::Msg>) -> bool {
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(message) = channel.wait().await {
            match message {
                russh::ChannelMsg::Failure => return true,
                russh::ChannelMsg::Success | russh::ChannelMsg::Data { .. } => return false,
                _ => {}
            }
        }
        true
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn a_shell_a_command_or_another_subsystem_is_refused() {
    let node = Node::start().await;
    let handle = node.ssh(USER_A).await.unwrap();

    let channel = handle.channel_open_session().await.unwrap();
    channel.request_shell(true).await.unwrap();
    assert!(refused(channel).await);

    let channel = handle.channel_open_session().await.unwrap();
    channel.exec(true, "id").await.unwrap();
    assert!(refused(channel).await);

    let channel = handle.channel_open_session().await.unwrap();
    channel.request_subsystem(true, "not-sftp").await.unwrap();
    assert!(refused(channel).await);
}

/// Envoi d'un fichier comme le font les clients : créé ou tronqué, écrit, fermé.
async fn put(sftp: &SftpSession, path: &str, data: &[u8]) -> Result<(), String> {
    let mut file = sftp.create(path).await.map_err(|e| e.to_string())?;
    file.write_all(data).await.map_err(|e| e.to_string())?;
    file.shutdown().await.map_err(|e| e.to_string())
}

/// Session avec le droit d'écrire (`servers.files.write`).
async fn writer(node: &Node) -> (russh::client::Handle<Client>, SftpSession) {
    node.api.write.store(true, Ordering::SeqCst);
    node.sftp(USER_A).await
}

fn uid_of(path: &Path) -> u32 {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(path).unwrap().uid()
}

#[tokio::test]
async fn an_upload_is_owned_by_the_service_and_journaled_at_close() {
    let node = Node::start().await;
    let (_handle, sftp) = writer(&node).await;

    let mut file = sftp.create("/world.zip").await.unwrap();
    file.write_all(&[7u8; 300_000]).await.unwrap();
    file.shutdown().await.unwrap();

    let host = node.host(node.a, "world.zip");
    assert_eq!(std::fs::read(&host).unwrap(), vec![7u8; 300_000]);
    // L'utilisateur du service (1000), comme une écriture du panel.
    assert_eq!(uid_of(&host), 1000);
    let written = node
        .api
        .wait_for(|e| matches!(e.event, SftpEventKind::FileWritten { .. }))
        .await;
    assert_eq!(
        written.event,
        SftpEventKind::FileWritten {
            path: "/app/data/world.zip".to_owned(),
            size: 300_000
        }
    );
}

#[tokio::test]
async fn a_file_written_by_the_panel_or_by_sftp_ends_up_the_same() {
    let node = Node::start().await;
    node.manager
        .file_write(node.a, "/app/data/web.txt", "same".to_owned(), None)
        .await
        .unwrap();
    let (_handle, sftp) = writer(&node).await;
    put(&sftp, "/sftp.txt", b"same").await.unwrap();

    let web = node.host(node.a, "web.txt");
    let over_sftp = node.host(node.a, "sftp.txt");
    assert_eq!(
        std::fs::read(&web).unwrap(),
        std::fs::read(&over_sftp).unwrap()
    );
    assert_eq!(uid_of(&web), uid_of(&over_sftp));
}

#[tokio::test]
async fn folders_renames_and_deletions_are_served_and_journaled() {
    let node = Node::start().await;
    let (_handle, sftp) = writer(&node).await;

    sftp.create_dir("/mods").await.unwrap();
    assert_eq!(uid_of(&node.host(node.a, "mods")), 1000);
    put(&sftp, "/mods/a.jar", b"jar").await.unwrap();
    // Un dossier non vide n'est pas supprimé d'un coup.
    assert!(sftp.remove_dir("/mods").await.is_err());
    sftp.rename("/mods/a.jar", "/mods/b.jar").await.unwrap();
    assert!(node.host(node.a, "mods/b.jar").exists());
    // `RENAME` n'écrase pas.
    put(&sftp, "/mods/c.jar", b"c").await.unwrap();
    assert!(sftp.rename("/mods/b.jar", "/mods/c.jar").await.is_err());
    sftp.remove_file("/mods/b.jar").await.unwrap();
    sftp.remove_file("/mods/c.jar").await.unwrap();
    sftp.remove_dir("/mods").await.unwrap();
    assert!(!node.host(node.a, "mods").exists());

    for expected in [
        SftpEventKind::DirectoryCreated {
            path: "/app/data/mods".to_owned(),
        },
        SftpEventKind::FileRenamed {
            from: "/app/data/mods/a.jar".to_owned(),
            to: "/app/data/mods/b.jar".to_owned(),
        },
        SftpEventKind::FileDeleted {
            path: "/app/data/mods".to_owned(),
        },
    ] {
        node.api.wait_for(|e| e.event == expected).await;
    }
}

#[tokio::test]
async fn writing_never_follows_a_link_nor_touches_kuro_files() {
    let node = Node::start().await;
    std::fs::write(node.host(node.b, "secret.txt"), "secret of b").unwrap();
    std::os::unix::fs::symlink(node.host(node.b, "secret.txt"), node.host(node.a, "link")).unwrap();
    std::os::unix::fs::symlink(node.host(node.b, ""), node.host(node.a, "other")).unwrap();
    let (_handle, sftp) = writer(&node).await;

    assert!(put(&sftp, "/link", b"overwritten").await.is_err());
    assert!(
        put(&sftp, "/other/secret.txt", b"overwritten")
            .await
            .is_err()
    );
    assert!(sftp.create_dir("/other/new").await.is_err());
    assert_eq!(
        std::fs::read_to_string(node.host(node.b, "secret.txt")).unwrap(),
        "secret of b"
    );
    // Supprimer le lien retire le lien seul.
    sftp.remove_file("/link").await.unwrap();
    assert!(node.host(node.b, "secret.txt").exists());
    // `..` reste dans la racine ; les fichiers temporaires de Kuro sont refusés.
    put(&sftp, "/../escape.txt", b"x").await.unwrap();
    assert!(node.host(node.a, "escape.txt").exists());
    assert!(put(&sftp, "/.kuro-tmp-x", b"x").await.is_err());
}

#[tokio::test]
async fn modes_lose_setuid_and_owners_cannot_be_changed() {
    let node = Node::start().await;
    let (_handle, sftp) = writer(&node).await;
    put(&sftp, "/start.sh", b"#!/bin/sh").await.unwrap();

    let mode = FileAttributes {
        permissions: Some(0o4755),
        ..FileAttributes::default()
    };
    sftp.set_metadata("/start.sh", mode).await.unwrap();
    use std::os::unix::fs::PermissionsExt;
    let actual = std::fs::metadata(node.host(node.a, "start.sh"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(actual & 0o7777, 0o755);

    let owner = FileAttributes {
        uid: Some(0),
        gid: Some(0),
        ..FileAttributes::default()
    };
    assert!(sftp.set_metadata("/start.sh", owner).await.is_err());
    assert_eq!(uid_of(&node.host(node.a, "start.sh")), 1000);
}

#[tokio::test]
async fn a_suspended_server_refuses_writes() {
    let node = Node::start().await;
    let (_handle, sftp) = writer(&node).await;
    node.manager.suspend(node.a, true).unwrap();
    // Une vérification accordée vaut deux secondes.
    tokio::time::sleep(Duration::from_millis(2100)).await;
    assert!(put(&sftp, "/late.txt", b"x").await.is_err());
    assert!(!node.host(node.a, "late.txt").exists());
}

#[tokio::test]
async fn a_withdrawn_write_permission_stops_writes_at_the_next_lease() {
    let node = Node::start().await;
    let (_handle, sftp) = writer(&node).await;
    put(&sftp, "/first.txt", b"x").await.unwrap();

    node.api.write.store(false, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(put(&sftp, "/second.txt", b"x").await.is_err());
    assert!(!node.host(node.a, "second.txt").exists());
    // Le refus est journalisé, la session reste ouverte en lecture.
    node.api
        .wait_for(|e| matches!(e.event, SftpEventKind::AccessDenied { .. }))
        .await;
    assert_eq!(sftp.read("/first.txt").await.unwrap(), b"x");
}

#[tokio::test]
async fn an_sftp_overwrite_keeps_the_previous_version_in_the_history() {
    let node = Node::start().await;
    std::fs::write(node.host(node.a, "ops.json"), "[\"alice\"]").unwrap();
    let (_handle, sftp) = writer(&node).await;
    put(&sftp, "/ops.json", b"[]").await.unwrap();

    let versions = node
        .manager
        .file_history(node.a, "/app/data/ops.json")
        .await
        .unwrap()
        .versions;
    assert_eq!(versions.len(), 1);
    assert_eq!(
        versions[0].channel,
        kuro_protocol::daemon::VersionChannel::Sftp
    );
    assert_eq!(versions[0].author.and_then(|author| author.id), Some(ADMIN));
    let kept = node
        .manager
        .file_version_read(node.a, "/app/data/ops.json", versions[0].id)
        .await
        .unwrap();
    assert_eq!(kept.content, "[\"alice\"]");
    // Le dossier `.kuro` du serveur (historique compris) reste hors d'atteinte : `..` ramène
    // à la racine du volume, où il n'existe pas.
    assert!(sftp.read_dir("/../.kuro").await.is_err());
}
