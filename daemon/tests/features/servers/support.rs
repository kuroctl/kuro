//! Spécifications de test (petites images, pas de jeu) et attente des états signalés.

use std::collections::BTreeMap;
use std::time::Duration;

use kuro_daemon::docker::Docker;
use kuro_daemon::server::{Manager, StateReceiver};
use kuro_protocol::DaemonMessage;
use kuro_protocol::api::server::{
    FileContent, FileFormat, FileSpec, LifecycleSpec, Limits, PowerAction, ReadySpec, ServerDeploy,
    ServiceSpec, StopSignal, StopSpec, VolumeSpec,
};
use kuro_protocol::daemon::BackupDone;
use kuro_protocol::daemon::server::ServerState;
use tempfile::TempDir;
use tokio::sync::mpsc;
use uuid::Uuid;

pub const IMAGE: &str = "docker.io/library/alpine:3.20";

/// Gestionnaire sur le Docker de la machine, avec ses messages à destination de l'API.
pub struct Node {
    pub manager: Manager,
    pub events: StateReceiver,
    pub streams: mpsc::Receiver<DaemonMessage>,
    pub backups: mpsc::Receiver<BackupDone>,
    pub data: TempDir,
}

pub fn node() -> Node {
    let data = TempDir::new().unwrap();
    let docker = Docker::connect(IMAGE).unwrap();
    let (manager, outgoing) = Manager::new(docker, data.path().to_path_buf());
    Node {
        manager,
        events: outgoing.events,
        streams: outgoing.streams,
        backups: outgoing.backups,
        data,
    }
}

pub fn service(name: &str, script: &str) -> ServiceSpec {
    ServiceSpec {
        name: name.to_owned(),
        image: IMAGE.to_owned(),
        command: vec!["sh".to_owned(), "-c".to_owned(), script.to_owned()],
        env: BTreeMap::new(),
        ports: Vec::new(),
        volumes: vec![VolumeSpec {
            name: "data".to_owned(),
            path: "/data".to_owned(),
        }],
        writable: vec!["/tmp".to_owned()],
        user: "1000:1000".to_owned(),
        limits: Limits {
            memory_bytes: 64 * 1024 * 1024,
            cpu_millis: 500,
            pids: 64,
        },
        depends_on: Vec::new(),
        healthcheck: None,
    }
}

/// Un service qui lit son fichier géré, annonce « ready » puis attend.
pub fn single(server_id: Uuid) -> ServerDeploy {
    ServerDeploy {
        server_id,
        services: vec![service(
            "app",
            "cat /data/app.properties; echo ready; exec sleep 3600",
        )],
        files: vec![FileSpec {
            service: "app".to_owned(),
            path: "/data/app.properties".to_owned(),
            format: FileFormat::Properties,
            content: FileContent::ManagedKeys {
                keys: BTreeMap::from([("motd".to_owned(), "hello".to_owned())]),
            },
        }],
        lifecycle: LifecycleSpec {
            service: "app".to_owned(),
            ready: Some(ReadySpec::Console("^ready$".to_owned())),
            // `sleep`, PID 1, ignore SIGTERM : l'arrêt passe par le délai de grâce puis SIGKILL.
            stop: StopSpec::Signal(StopSignal::Term),
            stop_timeout_secs: 2,
        },
        install: None,
        disk_bytes: 0,
        suspended: false,
        defer: false,
    }
}

/// Déploie puis démarre, jusqu'à l'état `running`.
pub async fn deploy_and_start(node: &mut Node, spec: ServerDeploy) {
    let id = spec.server_id;
    node.manager.deploy(spec).unwrap();
    wait_for(&mut node.events, id, ServerState::Offline).await;
    node.manager.power(id, PowerAction::Start).await.unwrap();
    wait_for(&mut node.events, id, ServerState::Running).await;
}

/// Attend un état ; échoue sur `failed` inattendu ou au bout du délai.
pub async fn wait_for(events: &mut StateReceiver, server_id: Uuid, state: ServerState) {
    let deadline = Duration::from_secs(120);
    let outcome = tokio::time::timeout(deadline, async {
        while let Some(report) = events.recv().await {
            if report.server_id != server_id {
                continue;
            }
            if report.state == state {
                return;
            }
            assert!(
                report.state != ServerState::Failed || state == ServerState::Failed,
                "server failed: {:?}",
                report.reason
            );
        }
        panic!("event channel closed");
    })
    .await;
    assert!(outcome.is_ok(), "timed out waiting for {state:?}");
}

/// Une commande refusée `Busy` est retentée : le gestionnaire finit l'opération précédente
/// (limites appliquées, suspension) en tâche de fond.
pub async fn until_free<T, F, Fut>(mut command: F) -> Result<T, kuro_daemon::server::CommandError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, kuro_daemon::server::CommandError>>,
{
    for _ in 0..200 {
        match command().await {
            Err(kuro_daemon::server::CommandError::Busy) => {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            outcome => return outcome,
        }
    }
    Err(kuro_daemon::server::CommandError::Busy)
}
