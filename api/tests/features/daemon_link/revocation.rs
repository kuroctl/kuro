//! Révocation et suppression d'un nœud connecté : coupure, refus de reconnexion, réenrôlement.

use std::time::Duration;

use kuro_api::audit::domain::AuditContext;
use kuro_api::enrollment::application::reissue_token;
use kuro_api::nodes::application::lifecycle;
use kuro_api::nodes::domain::NoLiveLinks;
use kuro_daemon::link::LinkError;
use tokio_util::sync::CancellationToken;

use crate::support::link::{TestApi, assert_closed, heartbeat, recv, say_hello, send};

fn context() -> AuditContext {
    AuditContext::cli("test")
}

#[tokio::test]
async fn revoking_cuts_the_live_session_and_refuses_reconnection() {
    let api = TestApi::start().await;
    let node = api.enroll_node("node-1").await;
    let mut ws = api.connect(&node).await;
    say_hello(&mut ws).await;

    lifecycle::revoke(
        &api.pool,
        &context(),
        api.links.as_ref(),
        node.id.as_uuid(),
        Some("host compromised".into()),
    )
    .await
    .unwrap();

    let notice = recv(&mut ws).await.unwrap();
    assert_eq!(notice["type"], "error");
    assert_eq!(notice["payload"]["code"], "node_revoked");
    assert_closed(&mut ws).await;
    assert_eq!(api.status(node.id).await, "revoked");

    let again = kuro_daemon::link::connect(&node.config).await.unwrap_err();
    assert!(matches!(again, LinkError::Revoked), "{again}");
}

#[tokio::test]
async fn a_revocation_decided_elsewhere_ends_the_session_at_the_next_heartbeat() {
    let api = TestApi::start().await;
    let node = api.enroll_node("node-1").await;
    let mut ws = api.connect(&node).await;
    say_hello(&mut ws).await;

    // Comme la CLI : aucune session dans ce processus.
    lifecycle::revoke(&api.pool, &context(), &NoLiveLinks, node.id.as_uuid(), None)
        .await
        .unwrap();
    send(&mut ws, heartbeat()).await;

    let notice = recv(&mut ws).await.unwrap();
    assert_eq!(notice["payload"]["code"], "node_revoked");
    assert_closed(&mut ws).await;
    assert_eq!(api.status(node.id).await, "revoked");
}

#[tokio::test]
async fn the_daemon_stops_for_good_when_revoked() {
    let api = TestApi::start().await;
    let node = api.enroll_node("node-1").await;
    let config = node.config.clone();
    let daemon = tokio::spawn(async move {
        kuro_daemon::link::run(&config, None, &CancellationToken::new()).await
    });
    api.wait_for_status(node.id, "online").await;

    lifecycle::revoke(
        &api.pool,
        &context(),
        api.links.as_ref(),
        node.id.as_uuid(),
        None,
    )
    .await
    .unwrap();

    let outcome = tokio::time::timeout(Duration::from_secs(10), daemon)
        .await
        .expect("daemon kept running after revocation")
        .unwrap();
    assert!(matches!(outcome, Err(LinkError::Revoked)), "{outcome:?}");
}

#[tokio::test]
async fn a_revoked_node_can_be_reenrolled_with_a_new_token() {
    let api = TestApi::start().await;
    let node = api.enroll_node("node-1").await;
    lifecycle::revoke(
        &api.pool,
        &context(),
        api.links.as_ref(),
        node.id.as_uuid(),
        None,
    )
    .await
    .unwrap();

    let ticket = reissue_token(
        &api.pool,
        &context(),
        node.id.as_uuid(),
        Duration::from_secs(900),
    )
    .await
    .unwrap();
    assert_eq!(api.status(node.id).await, "pending");
    api.enroll_from(
        "reinstalled-host",
        ticket.token.expose(),
        &api.pki.fingerprint,
    )
    .await
    .unwrap();

    assert_eq!(api.status(node.id).await, "offline");
    // L'ancien certificat est oublié : refusé comme inconnu, plus comme révoqué.
    let old = kuro_daemon::link::connect(&node.config).await.unwrap_err();
    assert!(!matches!(old, LinkError::Revoked), "{old}");
}

#[tokio::test]
async fn deleting_a_connected_node_disconnects_it() {
    let api = TestApi::start().await;
    let node = api.enroll_node("node-1").await;
    let mut ws = api.connect(&node).await;
    say_hello(&mut ws).await;

    lifecycle::delete(&api.pool, &context(), api.links.as_ref(), node.id.as_uuid())
        .await
        .unwrap();

    let notice = recv(&mut ws).await.unwrap();
    assert_eq!(notice["payload"]["code"], "node_revoked");
    assert_eq!(notice["payload"]["message"], "node deleted");
    assert_closed(&mut ws).await;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM nodes")
        .fetch_one(&api.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert!(kuro_daemon::link::connect(&node.config).await.is_err());
}
