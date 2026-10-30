//! Bout en bout : le vrai client du daemon se connecte, passe `online`, puis `offline`.

use kuro_api::nodes::application::Presence;
use kuro_api::nodes::infra::NodeRepository;
use tokio_util::sync::CancellationToken;

use crate::support::link::TestApi;

#[tokio::test]
async fn daemon_goes_online_then_offline_when_it_stops() {
    let api = TestApi::start().await;
    let node = api.enroll_node("node-1").await;
    let config = node.config.clone();
    let daemon_shutdown = CancellationToken::new();
    let daemon = tokio::spawn({
        let shutdown = daemon_shutdown.clone();
        async move { kuro_daemon::link::run(&config, None, &shutdown).await }
    });

    api.wait_for_status(node.id, "online").await;
    let (os, cpu_cores): (String, i32) =
        sqlx::query_as("SELECT os, cpu_cores FROM nodes WHERE id = $1")
            .bind(node.id.as_uuid())
            .fetch_one(&api.pool)
            .await
            .unwrap();
    assert_eq!(os, std::env::consts::OS);
    assert!(cpu_cores >= 1);

    daemon_shutdown.cancel();
    daemon.await.unwrap().unwrap();
    api.wait_for_status(node.id, "offline").await;
}

#[tokio::test]
async fn stale_online_nodes_are_reset_on_startup() {
    let api = TestApi::start().await;
    let node = api.enroll_node("node-1").await;
    sqlx::query("UPDATE nodes SET status = 'online' WHERE id = $1")
        .bind(node.id.as_uuid())
        .execute(&api.pool)
        .await
        .unwrap();

    let reset = Presence::new(NodeRepository::new(api.pool.clone()), api.instance.clone())
        .reset()
        .await
        .unwrap();

    assert_eq!(reset, 1);
    assert_eq!(api.status(node.id).await, "offline");
}
