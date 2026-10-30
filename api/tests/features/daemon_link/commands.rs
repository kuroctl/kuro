//! Les commandes de l'API ne bloquent pas le lien : pendant une opération longue, le daemon
//! continue d'envoyer ses heartbeats et répond aux autres commandes.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use kuro_api::servers::domain::{CommandFailure, NodeCommands};
use kuro_daemon::docker::Docker;
use kuro_daemon::fs::ServerDir;
use kuro_daemon::server::{Manager, Servers};
use kuro_protocol::ApiMessage;
use kuro_protocol::api::server::{
    FileContent, FileFormat, FileSpec, LifecycleSpec, Limits, ServerCommand, ServerDeploy,
    ServiceSpec, StopSignal, StopSpec, VolumeSpec,
};
use kuro_protocol::daemon::DaemonMessage;
use kuro_protocol::daemon::server::{CommandErrorCode, CommandResult};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::support::link::{HEARTBEAT_TIMEOUT_SECS, TestApi};

/// Un « Docker » qui accepte la connexion et ne répond jamais : toute commande qui le consulte
/// reste suspendue jusqu'au délai du client, bien au-delà du délai de heartbeat.
async fn silent_docker() -> (Docker, TcpListener) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = format!("http://{}", listener.local_addr().unwrap());
    let docker = Docker::connect_http(&addr, Duration::from_secs(120), "alpine:3.20").unwrap();
    (docker, listener)
}

/// Spécification minimale, écrite directement sur le disque du nœud (sans déploiement).
fn save_spec(data_root: &Path, server_id: Uuid) {
    let deploy = ServerDeploy {
        server_id,
        services: vec![ServiceSpec {
            name: "app".to_owned(),
            image: "alpine:3.20".to_owned(),
            command: vec!["sleep".to_owned(), "3600".to_owned()],
            env: BTreeMap::new(),
            ports: Vec::new(),
            volumes: vec![VolumeSpec {
                name: "data".to_owned(),
                path: "/data".to_owned(),
            }],
            writable: Vec::new(),
            user: "1000:1000".to_owned(),
            limits: Limits {
                memory_bytes: 64 * 1024 * 1024,
                cpu_millis: 500,
                pids: 64,
            },
            depends_on: Vec::new(),
            healthcheck: None,
        }],
        files: vec![FileSpec {
            service: "app".to_owned(),
            path: "/data/app.properties".to_owned(),
            format: FileFormat::Properties,
            content: FileContent::ManagedKeys {
                keys: BTreeMap::new(),
            },
        }],
        lifecycle: LifecycleSpec {
            service: "app".to_owned(),
            ready: None,
            stop: StopSpec::Signal(StopSignal::Term),
            stop_timeout_secs: 2,
        },
        install: None,
        disk_bytes: 0,
        suspended: false,
        defer: false,
        web: Vec::new(),
    };
    ServerDir::new(data_root, server_id)
        .save_spec(&deploy)
        .unwrap();
}

fn console_command(server_id: Uuid) -> ApiMessage {
    ApiMessage::ServerCommand(ServerCommand {
        server_id,
        line: "say hello".to_owned(),
    })
}

#[tokio::test]
async fn a_slow_command_does_not_block_heartbeats_nor_other_commands() {
    let api = TestApi::start().await;
    let node = api.enroll_node("node-1").await;
    let config = node.config.clone();
    let data_root = tempfile::TempDir::new().unwrap();
    let stuck = Uuid::new_v4();
    save_spec(data_root.path(), stuck);
    let (docker, _listener) = silent_docker().await;
    let (manager, outgoing) = Manager::new(docker, data_root.path().to_path_buf());

    let daemon_shutdown = CancellationToken::new();
    let daemon = tokio::spawn({
        let shutdown = daemon_shutdown.clone();
        async move {
            let mut servers = Servers { manager, outgoing };
            kuro_daemon::link::run(&config, Some(&mut servers), &shutdown).await
        }
    });
    api.wait_for_status(node.id, "online").await;

    // Suspendue : le daemon consulte son Docker muet pour savoir si le serveur tourne.
    let links = api.links.clone();
    let slow = tokio::spawn(async move { links.request(node.id, console_command(stuck)).await });

    // Pendant ce temps, une autre commande reçoit sa réponse tout de suite…
    let reply = tokio::time::timeout(
        Duration::from_secs(2),
        api.links.request(node.id, console_command(Uuid::new_v4())),
    )
    .await
    .expect("the link must keep answering during a slow command")
    .unwrap();
    assert!(
        matches!(
            reply,
            DaemonMessage::CommandResult(CommandResult::Error {
                code: CommandErrorCode::UnknownServer,
                ..
            })
        ),
        "unexpected reply: {reply:?}"
    );

    // … et le nœud reste `online` au-delà du délai de heartbeat.
    tokio::time::sleep(Duration::from_secs(HEARTBEAT_TIMEOUT_SECS + 1)).await;
    assert_eq!(api.status(node.id).await, "online");

    // La commande suspendue expire côté API sans avoir coupé le lien.
    assert!(matches!(slow.await.unwrap(), Err(CommandFailure::Timeout)));
    assert_eq!(api.status(node.id).await, "online");

    daemon_shutdown.cancel();
    daemon.await.unwrap().unwrap();
}
