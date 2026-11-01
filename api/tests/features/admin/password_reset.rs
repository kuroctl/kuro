//! Mot de passe oublié (ADR-0049) : lien à usage unique par e-mail, réponse qui ne révèle pas
//! les comptes, TOTP toujours exigé, verrouillage gardé.

use std::time::Duration;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

use crate::support::admin::{Account, PASSWORD, REFRESH_COOKIE, TestAdmin};
use crate::support::http::TestResponse;

const NEW_PASSWORD: &str = "a brand new passphrase";

/// SMTP réglé directement en base : une session admin ne s'ouvre pas sans TOTP quand il est
/// exigé, et le réglage par le REST est couvert par `mail.rs`.
async fn with_smtp(admin: &TestAdmin) {
    sqlx::query(
        "INSERT INTO mail_settings (host, port, security, from_address)
         VALUES ('smtp.example.com', 587, 'starttls', 'kuro@example.com')",
    )
    .execute(&admin.pool)
    .await
    .unwrap();
}

async fn forgot(admin: &TestAdmin, email: &str) -> TestResponse {
    admin
        .call(
            Method::POST,
            "/auth/password/forgot",
            None,
            Some(json!({ "email": email })),
        )
        .await
}

async fn reset(admin: &TestAdmin, token: &str, password: &str) -> TestResponse {
    admin
        .call(
            Method::POST,
            "/auth/password/reset",
            None,
            Some(json!({ "token": token, "password": password })),
        )
        .await
}

/// L'e-mail part en tâche de fond : attend le `count`-ième message.
async fn wait_for_mail(admin: &TestAdmin, count: usize) {
    for _ in 0..100 {
        if admin.mailer.sent().len() >= count {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!(
        "expected {count} e-mail(s), got {}",
        admin.mailer.sent().len()
    );
}

/// Jeton du lien du dernier e-mail, adressé à `account`.
fn last_link_token(admin: &TestAdmin, account: &Account) -> String {
    let message = admin.mailer.sent().pop().unwrap();
    assert_eq!(message.to, vec![account.email.clone()]);
    message
        .text
        .lines()
        .find_map(|line| line.strip_prefix("https://panel.example.com/password/reset/"))
        .unwrap()
        .to_owned()
}

/// Demande et attente du lien.
async fn link(admin: &TestAdmin, account: &Account, count: usize) -> String {
    let response = forgot(admin, &account.email).await;
    assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.json);
    wait_for_mail(admin, count).await;
    last_link_token(admin, account)
}

async fn last_reason(admin: &TestAdmin) -> Value {
    let event = admin.last_audit("auth.password_reset.requested").await;
    assert_eq!(event.outcome, "failure");
    event.details["reason"].clone()
}

#[tokio::test]
async fn a_forgotten_password_is_reset_once_through_the_emailed_link() {
    let admin = TestAdmin::start().await;
    with_smtp(&admin).await;
    let account = admin.account("alice@example.com", &[]).await;
    let session = admin.login(&account).await;

    let token = link(&admin, &account, 1).await;
    assert!(token.starts_with("kuro_pwd_"));
    let requested = admin.last_audit("auth.password_reset.requested").await;
    assert_eq!(requested.outcome, "success");
    assert_eq!(requested.target_id, Some(account.id.to_string()));

    let response = reset(&admin, &token, NEW_PASSWORD).await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(response.json["status"], "authenticated");
    assert!(response.cookie(REFRESH_COOKIE).is_some());
    let completed = admin.last_audit("auth.password_reset.completed").await;
    assert_eq!(completed.details["revoked_sessions"], 1);
    assert_eq!(completed.actor_id, Some(account.id));
    let closed = admin
        .call(Method::GET, "/me", Some(&session.access_token), None)
        .await;
    assert_eq!(closed.status, StatusCode::UNAUTHORIZED);
    let old = admin.login_with(&account.email, PASSWORD).await;
    assert_eq!(old.status, StatusCode::UNAUTHORIZED);
    let new = admin.login_with(&account.email, NEW_PASSWORD).await;
    assert_eq!(new.json["status"], "authenticated");
    let again = reset(&admin, &token, "yet another passphrase").await;
    assert_eq!(again.status, StatusCode::FORBIDDEN);
    assert_eq!(again.json["code"], "invalid_token");
    // Le jeton n'apparaît ni en base ni dans le journal.
    let stored: String = sqlx::query_scalar("SELECT token_hash FROM admin_password_resets")
        .fetch_one(&admin.pool)
        .await
        .unwrap();
    assert!(!stored.contains(&token));
    assert!(
        !admin
            .audit()
            .await
            .iter()
            .any(|row| row.details.to_string().contains("kuro_pwd_"))
    );
}

#[tokio::test]
async fn the_answer_never_reveals_whether_an_account_exists() {
    let admin = TestAdmin::start().await;
    with_smtp(&admin).await;
    let disabled = admin.account("bob@example.com", &[]).await;
    sqlx::query("UPDATE admin_users SET disabled_at = now() WHERE id = $1")
        .bind(disabled.id)
        .execute(&admin.pool)
        .await
        .unwrap();

    let unknown = forgot(&admin, "nobody@example.com").await;
    assert_eq!(unknown.status, StatusCode::ACCEPTED);
    assert_eq!(last_reason(&admin).await, "unknown_account");
    let refused = forgot(&admin, &disabled.email).await;
    assert_eq!(refused.status, StatusCode::ACCEPTED);
    assert_eq!(last_reason(&admin).await, "disabled");
    assert_eq!(unknown.body, refused.body);

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(admin.mailer.sent().is_empty());
}

#[tokio::test]
async fn nothing_is_sent_without_smtp() {
    let admin = TestAdmin::start().await;
    let account = admin.account("alice@example.com", &[]).await;

    let response = forgot(&admin, &account.email).await;

    assert_eq!(response.status, StatusCode::ACCEPTED);
    assert_eq!(last_reason(&admin).await, "mail_not_configured");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(admin.mailer.sent().is_empty());
}

#[tokio::test]
async fn requests_are_throttled_and_a_new_link_revokes_the_previous_one() {
    let admin = TestAdmin::start().await;
    with_smtp(&admin).await;
    let account = admin.account("alice@example.com", &[]).await;
    let first = link(&admin, &account, 1).await;

    let throttled = forgot(&admin, &account.email).await;
    assert_eq!(throttled.status, StatusCode::ACCEPTED);
    assert_eq!(last_reason(&admin).await, "too_soon");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(admin.mailer.sent().len(), 1);

    sqlx::query("UPDATE admin_password_resets SET created_at = created_at - interval '3 minutes'")
        .execute(&admin.pool)
        .await
        .unwrap();
    let second = link(&admin, &account, 2).await;

    assert_eq!(
        reset(&admin, &first, NEW_PASSWORD).await.status,
        StatusCode::FORBIDDEN
    );
    let response = reset(&admin, &second, NEW_PASSWORD).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
}

#[tokio::test]
async fn an_expired_link_is_refused() {
    let admin = TestAdmin::start().await;
    with_smtp(&admin).await;
    let account = admin.account("alice@example.com", &[]).await;
    let token = link(&admin, &account, 1).await;
    sqlx::query("UPDATE admin_password_resets SET expires_at = now() - interval '1 second'")
        .execute(&admin.pool)
        .await
        .unwrap();

    let response = reset(&admin, &token, NEW_PASSWORD).await;

    assert_eq!(response.status, StatusCode::FORBIDDEN);
    assert_eq!(response.json["code"], "invalid_token");
}

#[tokio::test]
async fn a_weak_password_is_refused_without_consuming_the_link() {
    let admin = TestAdmin::start().await;
    with_smtp(&admin).await;
    let account = admin.account("alice@example.com", &[]).await;
    let token = link(&admin, &account, 1).await;

    let weak = reset(&admin, &token, "short").await;

    assert_eq!(weak.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(weak.json["fields"][0]["field"], "password");
    assert_eq!(
        reset(&admin, &token, NEW_PASSWORD).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn totp_is_still_required_after_a_reset() {
    let admin = TestAdmin::with_totp(true).await;
    with_smtp(&admin).await;
    let account = admin.account("alice@example.com", &[]).await;
    let token = link(&admin, &account, 1).await;

    let response = reset(&admin, &token, NEW_PASSWORD).await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(response.json["status"], "totp_setup_required");
    assert!(response.json.get("access_token").is_none());
    assert!(response.cookie(REFRESH_COOKIE).is_none());
}

#[tokio::test]
async fn a_reset_does_not_lift_the_lockout() {
    let admin = TestAdmin::start().await;
    with_smtp(&admin).await;
    let account = admin.account("alice@example.com", &[]).await;
    let token = link(&admin, &account, 1).await;
    sqlx::query(
        "UPDATE admin_users SET locked_until = now() + interval '15 minutes' WHERE id = $1",
    )
    .bind(account.id)
    .execute(&admin.pool)
    .await
    .unwrap();

    let response = reset(&admin, &token, NEW_PASSWORD).await;

    // Le mot de passe est changé, mais la session attend la fin du verrouillage.
    assert_eq!(response.status, StatusCode::UNAUTHORIZED);
    admin.last_audit("auth.password_reset.completed").await;
    sqlx::query("UPDATE admin_users SET locked_until = NULL WHERE id = $1")
        .bind(account.id)
        .execute(&admin.pool)
        .await
        .unwrap();
    let login = admin.login_with(&account.email, NEW_PASSWORD).await;
    assert_eq!(login.json["status"], "authenticated");
}
