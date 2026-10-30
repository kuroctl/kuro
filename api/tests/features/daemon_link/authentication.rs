//! Seul un certificat signé par la CA et rattaché à un nœud connu ouvre le lien.

use crate::support::link::{TestApi, connect_without_client_cert};

#[tokio::test]
async fn client_without_certificate_is_rejected() {
    let api = TestApi::start().await;

    assert!(connect_without_client_cert(&api).await.is_err());
}

#[tokio::test]
async fn certificate_from_another_ca_is_rejected() {
    let api = TestApi::start().await;
    let files = api.pki.issue_foreign("rogue");

    let result = kuro_daemon::link::connect(&api.daemon_config(&files)).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn valid_certificate_of_unknown_node_is_rejected() {
    let api = TestApi::start().await;
    let files = api.pki.issue_unregistered("not-registered");

    let result = kuro_daemon::link::connect(&api.daemon_config(&files)).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn known_node_can_connect() {
    let api = TestApi::start().await;
    let node = api.enroll_node("node-1").await;

    let result = kuro_daemon::link::connect(&node.config).await;

    assert!(result.is_ok());
}
