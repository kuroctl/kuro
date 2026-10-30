//! Enrôlement par le vrai client du daemon (`kuro-daemonctl enroll`).

use kuro_daemon::enroll::EnrollError;
use tokio_util::sync::CancellationToken;

use crate::support::link::TestApi;

#[tokio::test]
async fn enrolled_node_connects_and_goes_online() {
    let api = TestApi::start().await;
    let ticket = api.add_node("node-1").await;

    let enrolled = api
        .enroll_from("host-1", ticket.token.expose(), &api.pki.fingerprint)
        .await
        .unwrap();

    assert_eq!(enrolled.node_id, ticket.node.id);
    assert_eq!(enrolled.node_name, "node-1");
    assert_eq!(api.status(ticket.node.id).await, "offline");

    let config = kuro_daemon::config::Config::load(&enrolled.config_path).unwrap();
    let shutdown = CancellationToken::new();
    let daemon = tokio::spawn({
        let shutdown = shutdown.clone();
        async move { kuro_daemon::link::run(&config, None, &shutdown).await }
    });
    api.wait_for_status(ticket.node.id, "online").await;
    shutdown.cancel();
    daemon.await.unwrap().unwrap();
}

#[tokio::test]
async fn token_is_single_use() {
    let api = TestApi::start().await;
    let ticket = api.add_node("node-1").await;
    let token = ticket.token.expose();
    api.enroll_from("host-1", token, &api.pki.fingerprint)
        .await
        .unwrap();

    let error = api
        .enroll_from("host-2", token, &api.pki.fingerprint)
        .await
        .unwrap_err();

    assert!(matches!(error, EnrollError::InvalidToken), "{error}");
}

#[tokio::test]
async fn concurrent_enrollments_with_the_same_token_succeed_once() {
    let api = TestApi::start().await;
    let ticket = api.add_node("node-1").await;
    let token = ticket.token.expose();

    let (first, second) = tokio::join!(
        api.enroll_from("host-1", token, &api.pki.fingerprint),
        api.enroll_from("host-2", token, &api.pki.fingerprint),
    );

    let successes = [first.is_ok(), second.is_ok()]
        .iter()
        .filter(|ok| **ok)
        .count();
    assert_eq!(successes, 1);
    let failure = first.err().or(second.err()).unwrap();
    assert!(matches!(failure, EnrollError::InvalidToken), "{failure}");
}

#[tokio::test]
async fn expired_token_is_rejected() {
    let api = TestApi::start().await;
    let ticket = api.add_node("node-1").await;
    sqlx::query("UPDATE enrollment_tokens SET expires_at = now() - interval '1 second'")
        .execute(&api.pool)
        .await
        .unwrap();

    let error = api
        .enroll_from("host-1", ticket.token.expose(), &api.pki.fingerprint)
        .await
        .unwrap_err();

    assert!(matches!(error, EnrollError::InvalidToken), "{error}");
    assert_eq!(api.status(ticket.node.id).await, "pending");
}

#[tokio::test]
async fn unknown_token_is_rejected() {
    let api = TestApi::start().await;
    api.add_node("node-1").await;

    let error = api
        .enroll_from("host-1", "kuro_enr_not-a-real-token", &api.pki.fingerprint)
        .await
        .unwrap_err();

    assert!(matches!(error, EnrollError::InvalidToken), "{error}");
}

#[tokio::test]
async fn wrong_ca_fingerprint_never_sends_the_token() {
    let api = TestApi::start().await;
    let ticket = api.add_node("node-1").await;
    let wrong = "0".repeat(64);

    let error = api
        .enroll_from("host-1", ticket.token.expose(), &wrong)
        .await
        .unwrap_err();

    assert!(matches!(error, EnrollError::Io(_)), "{error}");
    // Le token n'a pas été présenté : il reste utilisable avec la bonne empreinte.
    api.enroll_from("host-2", ticket.token.expose(), &api.pki.fingerprint)
        .await
        .unwrap();
}

#[tokio::test]
async fn malformed_fingerprint_is_rejected_before_any_connection() {
    let api = TestApi::start().await;

    let error = api
        .enroll_from("host-1", "kuro_enr_x", "not-a-fingerprint")
        .await
        .unwrap_err();

    assert!(matches!(error, EnrollError::InvalidFingerprint));
}

#[tokio::test]
async fn already_enrolled_host_is_never_overwritten() {
    let api = TestApi::start().await;
    let first = api.add_node("node-1").await;
    api.enroll_from("host-1", first.token.expose(), &api.pki.fingerprint)
        .await
        .unwrap();
    let key_path = api.host_config_path("host-1").with_file_name("node.key");
    let key_before = std::fs::read_to_string(&key_path).unwrap();

    let second = api.add_node("node-2").await;
    let error = api
        .enroll_from("host-1", second.token.expose(), &api.pki.fingerprint)
        .await
        .unwrap_err();

    assert!(matches!(error, EnrollError::AlreadyEnrolled(_)), "{error}");
    assert_eq!(std::fs::read_to_string(&key_path).unwrap(), key_before);
    // Le token du second nœud n'a pas été consommé.
    assert_eq!(api.status(second.node.id).await, "pending");
}

#[tokio::test]
async fn enrollment_is_audited() {
    let api = TestApi::start().await;
    let ticket = api.add_node("node-1").await;
    api.enroll_from("host-1", "kuro_enr_not-a-real-token", &api.pki.fingerprint)
        .await
        .unwrap_err();
    api.enroll_from("host-2", ticket.token.expose(), &api.pki.fingerprint)
        .await
        .unwrap();

    let events: Vec<(String, String, String, Option<uuid::Uuid>)> =
        sqlx::query_as("SELECT action, outcome, actor_kind, actor_id FROM audit_log ORDER BY seq")
            .fetch_all(&api.pool)
            .await
            .unwrap();

    let node_id = Some(ticket.node.id);
    assert_eq!(
        events,
        [
            ("node.added".into(), "success".into(), "cli".into(), None),
            (
                "node.enrolled".into(),
                "failure".into(),
                "anonymous".into(),
                None
            ),
            (
                "node.enrolled".into(),
                "success".into(),
                "node".into(),
                node_id
            ),
        ]
    );
    let details: Vec<String> = sqlx::query_scalar("SELECT details::text FROM audit_log")
        .fetch_all(&api.pool)
        .await
        .unwrap();
    assert!(!details.concat().contains(ticket.token.expose()));
}

#[tokio::test]
async fn reissued_token_replaces_the_previous_one() {
    let api = TestApi::start().await;
    let first = api.add_node("node-1").await;

    let second = kuro_api::enrollment::application::reissue_token(
        &api.pool,
        &kuro_api::audit::domain::AuditContext::cli("test"),
        first.node.id,
        std::time::Duration::from_secs(900),
    )
    .await
    .unwrap();

    let old = api
        .enroll_from("host-1", first.token.expose(), &api.pki.fingerprint)
        .await
        .unwrap_err();
    assert!(matches!(old, EnrollError::InvalidToken), "{old}");
    api.enroll_from("host-2", second.token.expose(), &api.pki.fingerprint)
        .await
        .unwrap();
    assert_eq!(api.status(first.node.id).await, "offline");
}
