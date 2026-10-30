//! `add_node` : le cas d'usage derrière `kuro-api node add` (et, demain, le REST du panel).

use std::time::Duration;

use kuro_api::audit::domain::AuditContext;
use kuro_api::enrollment::application::{AddNodeError, add_node};
use kuro_api::nodes::application::PlacementInput;

use crate::support::db;

const TTL: Duration = Duration::from_secs(900);

#[tokio::test]
async fn added_node_is_pending_with_a_single_use_token() {
    let pool = db::fresh_pool().await;

    let ticket = add_node(
        &pool,
        &AuditContext::cli("test"),
        "node-1",
        PlacementInput::default(),
        TTL,
    )
    .await
    .unwrap();

    assert_eq!(ticket.node.status, "pending");
    let token = ticket.token.expose();
    assert!(token.starts_with("kuro_enr_"));
    assert_eq!(token.len(), "kuro_enr_".len() + 64);
    let status: String = sqlx::query_scalar("SELECT status FROM nodes WHERE id = $1")
        .bind(ticket.node.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "pending");
}

#[tokio::test]
async fn token_is_stored_hashed_only() {
    let pool = db::fresh_pool().await;

    let ticket = add_node(
        &pool,
        &AuditContext::cli("test"),
        "node-1",
        PlacementInput::default(),
        TTL,
    )
    .await
    .unwrap();

    let stored: String = sqlx::query_scalar("SELECT token_hash FROM enrollment_tokens")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_ne!(stored, ticket.token.expose());
    assert!(!stored.contains(ticket.token.expose()));
    assert!(!format!("{:?}", ticket.token).contains(ticket.token.expose()));
}

#[tokio::test]
async fn invalid_node_names_are_rejected() {
    let pool = db::fresh_pool().await;

    for name in [
        "",
        "Node",
        "-node",
        "node-",
        "node_1",
        "nœud",
        &"a".repeat(64),
    ] {
        let error = add_node(
            &pool,
            &AuditContext::cli("test"),
            name,
            PlacementInput::default(),
            TTL,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(error, AddNodeError::InvalidName(_)),
            "{name:?} was accepted"
        );
    }
}

#[tokio::test]
async fn duplicate_name_is_rejected_without_leaving_a_token() {
    let pool = db::fresh_pool().await;
    add_node(
        &pool,
        &AuditContext::cli("test"),
        "node-1",
        PlacementInput::default(),
        TTL,
    )
    .await
    .unwrap();

    let error = add_node(
        &pool,
        &AuditContext::cli("test"),
        "node-1",
        PlacementInput::default(),
        TTL,
    )
    .await
    .unwrap_err();

    assert!(matches!(error, AddNodeError::AlreadyExists));
    let tokens: i64 = sqlx::query_scalar("SELECT count(*) FROM enrollment_tokens")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(tokens, 1);
}
