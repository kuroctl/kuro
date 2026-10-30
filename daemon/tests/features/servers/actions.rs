//! Actions d'un prefab (ADR-0039) sur le Docker de la machine : commande exécutée dans le
//! conteneur du service, avec son utilisateur ; refusée arrêtée ou vers un service inconnu.

use kuro_daemon::server::CommandError;
use kuro_protocol::api::server::{PowerAction, ServerAction};
use kuro_protocol::daemon::server::ServerState;
use uuid::Uuid;

use super::support::{deploy_and_start, node, single, wait_for};

fn action(server_id: Uuid, service: &str, command: &[&str]) -> ServerAction {
    ServerAction {
        server_id,
        action: "test".to_owned(),
        service: service.to_owned(),
        command: command.iter().map(|s| (*s).to_owned()).collect(),
        timeout_secs: 30,
    }
}

#[tokio::test]
async fn an_action_runs_in_the_service_container_as_its_user() {
    let mut node = node();
    let id = Uuid::now_v7();
    deploy_and_start(&mut node, single(id)).await;

    let result = node
        .manager
        .action(action(
            id,
            "app",
            &[
                "sh",
                "-c",
                "id -u; echo to-stderr >&2; cat /data/app.properties; exit 3",
            ],
        ))
        .await
        .unwrap();

    assert_eq!(result.server_id, id);
    assert_eq!(result.exit_code, 3);
    assert!(result.output.contains("1000"), "{}", result.output);
    assert!(result.output.contains("to-stderr"), "{}", result.output);
    assert!(result.output.contains("motd=hello"), "{}", result.output);
    assert!(!result.truncated);

    // Sortie bornée.
    let long = node
        .manager
        .action(action(id, "app", &["sh", "-c", "yes | head -c 20000"]))
        .await
        .unwrap();
    assert!(long.truncated);
    assert!(long.output.len() <= 4096);

    // Service inconnu : la spécification ne le déclare pas.
    assert!(matches!(
        node.manager.action(action(id, "nope", &["true"])).await,
        Err(CommandError::InvalidSpec(_))
    ));

    // Arrêté : rien à exécuter.
    node.manager.power(id, PowerAction::Stop).await.unwrap();
    wait_for(&mut node.events, id, ServerState::Offline).await;
    assert!(matches!(
        node.manager.action(action(id, "app", &["true"])).await,
        Err(CommandError::NotRunning)
    ));

    node.manager.delete(id).unwrap();
    wait_for(&mut node.events, id, ServerState::Deleted).await;
}
