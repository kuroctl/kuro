//! Plusieurs instances de l'API sur la même base (ADR-0037) : présence tenue par instance,
//! commandes relayées à l'instance qui tient le nœud, tickets consommables partout, console
//! refusée tant que les flux ne traversent pas les instances.

use std::sync::Arc;

use axum::http::{Method, StatusCode};
use kuro_api::cluster::Bus;
use kuro_api::nodes::application::{Instance, Presence};
use kuro_api::nodes::domain::NodeId;
use kuro_api::nodes::infra::NodeRepository;
use kuro_api::servers::application::tickets;
use kuro_api::servers::domain::{CommandFailure, NodeCommands};
use kuro_api::ws::{CommandRelay, LinkRegistry};
use kuro_protocol::ApiMessage;
use kuro_protocol::api::server::{PowerAction, ServerPower};
use kuro_protocol::daemon::DaemonMessage;
use kuro_protocol::daemon::server::CommandResult;
use serde_json::json;
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::support::admin::TestAdmin;
use crate::support::link::TestApi;
use crate::support::platform::Platform;

/// Relais et bus d'une instance, arrêtés avec elle.
struct Relayed {
    relay: Arc<CommandRelay>,
    shutdown: CancellationToken,
}

impl Drop for Relayed {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

/// L'instance du listener de test, avec son registre (le nœud y est relié).
async fn relayed(api: &TestApi) -> Relayed {
    relay(api.pool.clone(), api.instance.clone(), api.links.clone()).await
}

/// Une autre instance sur la même base : registre vide, aucun nœud relié.
async fn other_instance(pool: &PgPool) -> Relayed {
    let instance = Instance::start(pool.clone()).await.unwrap();
    relay(pool.clone(), instance, Arc::new(LinkRegistry::default())).await
}

async fn relay(pool: PgPool, instance: Instance, registry: Arc<LinkRegistry>) -> Relayed {
    let relay = CommandRelay::new(registry, pool.clone(), instance.id);
    let shutdown = CancellationToken::new();
    tokio::spawn(
        relay
            .subscribe(Bus::new(pool))
            .run_forever(shutdown.clone()),
    );
    // Le bus écoute avant que le test ne dépose sa première commande.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    Relayed { relay, shutdown }
}

fn power(server_id: Uuid) -> ApiMessage {
    ApiMessage::ServerPower(ServerPower {
        server_id,
        action: PowerAction::Start,
    })
}

async fn forwarded_commands(pool: &PgPool, node: NodeId) -> i64 {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM node_commands
           WHERE node_id = $1 AND replied_at IS NOT NULL"#,
        node.as_uuid()
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn a_command_for_a_node_held_by_another_instance_reaches_it() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565").await;
    let node = daemon.node.id;
    let _holder = relayed(&platform.api).await;
    let other = other_instance(&platform.api.pool).await;

    // Depuis l'autre instance : déposée en base, envoyée par l'instance qui tient le nœud.
    let (reply, command) = tokio::join!(other.relay.request(node, power(Uuid::new_v4())), async {
        daemon.accept().await
    });
    assert_eq!(command["type"], "server.power");
    assert_eq!(
        reply.unwrap(),
        DaemonMessage::CommandResult(CommandResult::Ok)
    );
    assert_eq!(forwarded_commands(&platform.api.pool, node).await, 1);

    // Depuis l'instance qui tient le nœud : en direct, rien en base.
    let (reply, _) = tokio::join!(
        _holder.relay.request(node, power(Uuid::new_v4())),
        daemon.accept()
    );
    assert_eq!(
        reply.unwrap(),
        DaemonMessage::CommandResult(CommandResult::Ok)
    );
    assert_eq!(forwarded_commands(&platform.api.pool, node).await, 1);

    // Nœud relié nulle part : hors ligne, sans attendre.
    let unknown = NodeId::from(Uuid::new_v4());
    assert_eq!(
        other.relay.request(unknown, power(Uuid::new_v4())).await,
        Err(CommandFailure::Offline)
    );
}

#[tokio::test]
async fn another_instance_starting_keeps_my_nodes_online_until_i_die() {
    let platform = Platform::start().await;
    let daemon = platform.online_node("node-1", "25565").await;
    let node = daemon.node.id;
    let pool = platform.api.pool.clone();
    let other = Instance::start(pool.clone()).await.unwrap();

    // Démarrage de l'autre instance : le nœud, tenu par une instance vivante, reste en ligne.
    let reset = Presence::new(NodeRepository::new(pool.clone()), other.clone())
        .reset()
        .await
        .unwrap();
    assert_eq!(reset, 0);
    assert_eq!(platform.api.status(node).await, "online");

    // L'instance du listener cesse de battre : au balayage suivant, ses nœuds passent hors
    // ligne et sa ligne disparaît.
    sqlx::query(
        "UPDATE api_instances SET last_seen_at = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(platform.api.instance.id.as_uuid())
    .execute(&pool)
    .await
    .unwrap();
    let swept = other.sweep().await.unwrap();
    assert_eq!((swept.instances, swept.nodes), (1, 1));
    assert_eq!(platform.api.status(node).await, "offline");
    drop(daemon);
}

#[tokio::test]
async fn a_ticket_is_consumed_once_for_its_server() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565").await;
    let (server, _) = platform.deploy_minecraft(&mut daemon, json!({})).await;
    let id: Uuid = server["id"].as_str().unwrap().parse().unwrap();
    let pool = &platform.api.pool;

    let response = platform
        .call(Method::POST, &format!("/servers/{id}/console/ticket"), None)
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.json);
    let ticket = response.json["ticket"].as_str().unwrap().to_owned();

    // N'importe quelle instance le consomme (la base est le seul état) ; une seule fois.
    assert!(
        tickets::redeem_console(pool, &ticket, id)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        tickets::redeem_console(pool, &ticket, id)
            .await
            .unwrap()
            .is_none()
    );

    // Présenté pour un autre serveur : consommé, mais refusé.
    let response = platform
        .call(Method::POST, &format!("/servers/{id}/console/ticket"), None)
        .await;
    let ticket = response.json["ticket"].as_str().unwrap().to_owned();
    assert!(
        tickets::redeem_console(pool, &ticket, Uuid::new_v4())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        tickets::redeem_console(pool, &ticket, id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn the_console_of_a_node_held_elsewhere_is_refused() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565").await;
    let (server, _) = platform.deploy_minecraft(&mut daemon, json!({})).await;
    let id = server["id"].as_str().unwrap().to_owned();
    let path = format!("/servers/{id}/console/ticket");

    // Le REST d'une autre instance, sur la même base : aucun nœud relié à son registre.
    let elsewhere = TestAdmin::on(
        platform.api.pool.clone(),
        Arc::new(LinkRegistry::default()),
        platform.api.console.clone(),
        false,
    );
    let refused = elsewhere
        .call(Method::POST, &path, Some(&platform.token), None)
        .await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.json);
    assert_eq!(refused.json["code"], "console_held_elsewhere");

    // Depuis l'instance à laquelle le nœud est relié : la console s'ouvre.
    let allowed = platform.call(Method::POST, &path, None).await;
    assert_eq!(allowed.status, StatusCode::CREATED, "{}", allowed.json);
}
