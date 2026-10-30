//! Cycle de vie complet sur le Docker de la machine (requis, comme Postgres pour l'API) :
//! déploiement, démarrage, durcissement des conteneurs, arrêt, suppression.

use bollard::query_parameters::ListContainersOptionsBuilder;
use kuro_daemon::docker::{container_name, network_name};
use kuro_daemon::server::CommandError;
use kuro_protocol::api::server::{PowerAction, ReadySpec};
use kuro_protocol::daemon::server::ServerState;
use uuid::Uuid;

use super::support::{deploy_and_start, node, service, single, wait_for};

fn bollard() -> bollard::Docker {
    bollard::Docker::connect_with_defaults().unwrap()
}

/// Sortie d'une commande lancée dans un conteneur en marche, sans espaces de fin.
async fn exec_output(container: &str, cmd: &[&str]) -> String {
    use bollard::exec::StartExecResults;
    use bollard::models::ExecConfig;
    use futures_util::StreamExt;

    let docker = bollard();
    let exec = docker
        .create_exec(
            container,
            ExecConfig {
                cmd: Some(cmd.iter().map(|part| (*part).to_owned()).collect()),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let mut text = String::new();
    if let StartExecResults::Attached { mut output, .. } =
        docker.start_exec(&exec.id, None).await.unwrap()
    {
        while let Some(chunk) = output.next().await {
            text.push_str(&chunk.unwrap().to_string());
        }
    }
    text.trim_end().to_owned()
}

#[tokio::test]
async fn a_server_is_deployed_started_stopped_and_deleted() {
    let mut node = node();
    let id = Uuid::now_v7();

    deploy_and_start(&mut node, single(id)).await;

    // Fichier géré écrit avant le démarrage, sous le volume du service.
    let properties = node
        .data
        .path()
        .join(id.to_string())
        .join("volumes/app/data/app.properties");
    assert_eq!(
        std::fs::read_to_string(&properties).unwrap(),
        "motd=hello\n"
    );

    // Durcissement appliqué quelle que soit la spécification.
    let inspect = bollard()
        .inspect_container(&container_name(id, "app"), None)
        .await
        .unwrap();
    let config = inspect.config.unwrap();
    let host = inspect.host_config.unwrap();
    assert_eq!(config.user.as_deref(), Some("1000:1000"));
    assert_eq!(host.readonly_rootfs, Some(true));
    assert_eq!(host.cap_drop, Some(vec!["ALL".to_owned()]));
    assert_eq!(
        host.security_opt,
        Some(vec!["no-new-privileges".to_owned()])
    );
    assert_eq!(host.memory, Some(64 * 1024 * 1024));
    assert_eq!(host.memory_swap, Some(64 * 1024 * 1024));
    assert_eq!(host.pids_limit, Some(64));
    assert_eq!(host.nano_cpus, Some(500_000_000));
    assert_eq!(
        host.network_mode.as_deref(),
        Some(network_name(id).as_str())
    );
    assert_eq!(
        host.tmpfs.unwrap()["/tmp"],
        "rw,exec,nosuid,nodev,uid=1000,gid=1000,mode=0755"
    );
    // Le chemin `writable` appartient vraiment au service, et n'est pas modifiable par tous.
    assert_eq!(
        exec_output(
            &container_name(id, "app"),
            &["stat", "-c", "%u:%g %a", "/tmp"]
        )
        .await,
        "1000:1000 755"
    );

    node.manager.power(id, PowerAction::Stop).await.unwrap();
    wait_for(&mut node.events, id, ServerState::Stopping).await;
    wait_for(&mut node.events, id, ServerState::Offline).await;

    node.manager.delete(id).unwrap();
    wait_for(&mut node.events, id, ServerState::Deleted).await;
    assert!(
        bollard()
            .inspect_container(&container_name(id, "app"), None)
            .await
            .is_err()
    );
    assert!(
        bollard()
            .inspect_network(&network_name(id), None)
            .await
            .is_err()
    );
    assert!(!node.data.path().join(id.to_string()).exists());
}

#[tokio::test]
async fn services_reach_each_other_by_name() {
    let mut node = node();
    let id = Uuid::now_v7();
    let mut spec = single(id);
    spec.files.clear();
    spec.services = vec![service("db", "exec sleep 3600"), {
        let mut web = service(
            "web",
            // `getent` passe par le résolveur système, comme une vraie application ;
            // `nslookup` (busybox) échoue sur un nom court quand l'hôte a des domaines de
            // recherche DNS (runners GitHub).
            "until getent hosts db >/dev/null 2>&1; do sleep 1; done; echo ready; exec sleep 3600",
        );
        web.depends_on = vec!["db".to_owned()];
        web
    }];
    spec.lifecycle.service = "web".to_owned();
    spec.lifecycle.ready = Some(ReadySpec::Console("^ready$".to_owned()));

    deploy_and_start(&mut node, spec).await;

    node.manager.power(id, PowerAction::Kill).await.unwrap();
    wait_for(&mut node.events, id, ServerState::Offline).await;
    node.manager.delete(id).unwrap();
    wait_for(&mut node.events, id, ServerState::Deleted).await;
}

#[tokio::test]
async fn one_operation_at_a_time_per_server() {
    let mut node = node();
    let id = Uuid::now_v7();

    node.manager.deploy(single(id)).unwrap();
    let second = node.manager.deploy(single(id));

    assert_eq!(second, Err(CommandError::Busy));
    wait_for(&mut node.events, id, ServerState::Offline).await;
    node.manager.delete(id).unwrap();
    wait_for(&mut node.events, id, ServerState::Deleted).await;
}

#[tokio::test]
async fn an_unknown_server_is_refused() {
    let node = node();

    assert_eq!(
        node.manager.power(Uuid::now_v7(), PowerAction::Start).await,
        Err(CommandError::UnknownServer)
    );
}

#[tokio::test]
async fn a_root_service_is_never_created() {
    let node = node();
    let mut spec = single(Uuid::now_v7());
    spec.services[0].user = "0:0".to_owned();

    let refused = node.manager.deploy(spec.clone());

    assert!(matches!(refused, Err(CommandError::InvalidSpec(_))));
    let filters = std::collections::HashMap::from([(
        "label",
        vec![format!("kuro.server_id={}", spec.server_id)],
    )]);
    let options = ListContainersOptionsBuilder::new()
        .all(true)
        .filters(&filters)
        .build();
    assert!(
        bollard()
            .list_containers(Some(options))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_deferred_deploy_keeps_a_running_server_until_its_next_restart() {
    let mut node = node();
    let id = Uuid::now_v7();
    deploy_and_start(&mut node, single(id)).await;
    let container = || async {
        bollard()
            .inspect_container(&container_name(id, "app"), None)
            .await
            .unwrap()
    };
    let before = container().await;
    let pending = node.data.path().join(id.to_string()).join(".kuro/pending");

    // Nouveau contenu sur un serveur en marche : il tourne toujours, rien n'est recréé.
    let mut changed = single(id);
    changed.services[0]
        .env
        .insert("KURO_MODS".to_owned(), "more-mobs".to_owned());
    changed.defer = true;
    node.manager.deploy(changed).unwrap();
    for _ in 0..50 {
        if pending.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(pending.exists(), "the deferred spec is marked");
    let during = container().await;
    assert_eq!(during.id, before.id, "the container is not recreated");
    assert!(during.state.unwrap().running.unwrap());

    // Au redémarrage, la spécification gardée est appliquée.
    node.manager.power(id, PowerAction::Restart).await.unwrap();
    wait_for(&mut node.events, id, ServerState::Running).await;
    let after = container().await;
    assert_ne!(after.id, before.id, "recreated with the new spec");
    let env = after.config.unwrap().env.unwrap();
    assert!(env.iter().any(|e| e == "KURO_MODS=more-mobs"), "{env:?}");
    assert!(!pending.exists());

    node.manager.delete(id).unwrap();
    wait_for(&mut node.events, id, ServerState::Deleted).await;
}

/// Une commande `server.deploy` rejouée (reconnexion, double envoi) avec la même spécification
/// ne redémarre pas un serveur en marche.
#[tokio::test]
async fn redeploying_the_same_spec_keeps_a_running_server_running() {
    let mut node = node();
    let id = Uuid::now_v7();
    deploy_and_start(&mut node, single(id)).await;
    let started = bollard()
        .inspect_container(&container_name(id, "app"), None)
        .await
        .unwrap()
        .state
        .and_then(|state| state.started_at);

    node.manager.deploy(single(id)).unwrap();

    // Ni `stopping` ni `offline` avant `running`, et le conteneur est le même.
    let mut seen = Vec::new();
    loop {
        let report = tokio::time::timeout(std::time::Duration::from_secs(30), node.events.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(report.server_id, id);
        seen.push(report.state);
        if report.state == ServerState::Running {
            break;
        }
    }
    assert!(
        seen.iter()
            .all(|state| matches!(state, ServerState::Deploying | ServerState::Running)),
        "{seen:?}"
    );
    let still = bollard()
        .inspect_container(&container_name(id, "app"), None)
        .await
        .unwrap()
        .state
        .and_then(|state| state.started_at);
    assert_eq!(still, started);

    node.manager.delete(id).unwrap();
    wait_for(&mut node.events, id, ServerState::Deleted).await;
}
