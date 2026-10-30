//! TOTP obligatoire : configuration au premier accès, vérification, anti-rejeu.

use axum::http::{Method, StatusCode};
use kuro_api::auth::totp;
use serde_json::{Value, json};
use totp_rs::Secret;

use crate::support::admin::{PASSWORD, REFRESH_COOKIE, TestAdmin};

fn now() -> u64 {
    u64::try_from(time::OffsetDateTime::now_utc().unix_timestamp()).unwrap()
}

fn code(secret: &[u8], at: u64) -> String {
    totp::current_code(secret, at).unwrap()
}

/// Mot de passe, configuration et confirmation du TOTP ; retourne le secret.
async fn enroll_totp(admin: &TestAdmin, email: &str) -> Vec<u8> {
    let login = admin.login_with(email, PASSWORD).await;
    assert_eq!(
        login.json["status"], "totp_setup_required",
        "{}",
        login.json
    );
    let challenge = login.json["challenge"].as_str().unwrap().to_owned();

    let setup = admin
        .call(
            Method::POST,
            "/auth/totp/setup",
            None,
            Some(json!({ "challenge": challenge })),
        )
        .await;
    assert_eq!(setup.status, StatusCode::OK, "{}", setup.json);
    assert!(
        setup.json["otpauth_uri"]
            .as_str()
            .unwrap()
            .starts_with("otpauth://totp/")
    );
    let secret = Secret::try_from_base32(setup.json["secret"].as_str().unwrap())
        .unwrap()
        .as_bytes()
        .to_vec();

    let confirm = admin
        .call(
            Method::POST,
            "/auth/totp/confirm",
            None,
            Some(json!({ "challenge": challenge, "code": code(&secret, now()) })),
        )
        .await;
    assert_eq!(confirm.status, StatusCode::OK, "{}", confirm.json);
    assert!(confirm.cookie(REFRESH_COOKIE).is_some());
    secret
}

async fn totp_challenge(admin: &TestAdmin, email: &str) -> String {
    let login = admin.login_with(email, PASSWORD).await;
    assert_eq!(login.json["status"], "totp_required", "{}", login.json);
    login.json["challenge"].as_str().unwrap().to_owned()
}

async fn verify(admin: &TestAdmin, challenge: &str, code: &str) -> (StatusCode, Value) {
    let response = admin
        .call(
            Method::POST,
            "/auth/totp/verify",
            None,
            Some(json!({ "challenge": challenge, "code": code })),
        )
        .await;
    (response.status, response.json)
}

#[tokio::test]
async fn totp_is_set_up_at_first_login_when_required() {
    let admin = TestAdmin::with_totp(true).await;
    let account = admin.account("alice@example.com", &[]).await;

    enroll_totp(&admin, &account.email).await;

    admin.last_audit("auth.totp.enabled").await;
    let login = admin.last_audit("auth.login.succeeded").await;
    assert_eq!(login.details["mfa"], "totp");
}

#[tokio::test]
async fn password_alone_never_opens_a_session_when_totp_is_required() {
    let admin = TestAdmin::with_totp(true).await;
    let account = admin.account("alice@example.com", &[]).await;

    let login = admin.login_with(&account.email, PASSWORD).await;

    assert!(login.json.get("access_token").is_none());
    assert!(login.cookie(REFRESH_COOKIE).is_none());
    let challenge = login.json["challenge"].as_str().unwrap();
    let as_bearer = admin.call(Method::GET, "/me", Some(challenge), None).await;
    assert_eq!(as_bearer.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn configured_totp_is_required_at_each_login() {
    let admin = TestAdmin::with_totp(true).await;
    let account = admin.account("alice@example.com", &[]).await;
    let secret = enroll_totp(&admin, &account.email).await;

    let challenge = totp_challenge(&admin, &account.email).await;
    // Code du pas suivant : celui du pas courant a déjà servi à la confirmation.
    let (status, body) = verify(&admin, &challenge, &code(&secret, now() + 30)).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["access_token"].is_string());
}

#[tokio::test]
async fn totp_codes_cannot_be_replayed() {
    let admin = TestAdmin::with_totp(true).await;
    let account = admin.account("alice@example.com", &[]).await;
    let secret = enroll_totp(&admin, &account.email).await;
    let used = code(&secret, now() + 30);
    let challenge = totp_challenge(&admin, &account.email).await;
    verify(&admin, &challenge, &used).await;

    let challenge = totp_challenge(&admin, &account.email).await;
    let (status, body) = verify(&admin, &challenge, &used).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], "invalid_code");
}

#[tokio::test]
async fn wrong_totp_code_is_rejected_and_audited() {
    let admin = TestAdmin::with_totp(true).await;
    let account = admin.account("alice@example.com", &[]).await;
    let secret = enroll_totp(&admin, &account.email).await;
    let challenge = totp_challenge(&admin, &account.email).await;
    let wrong = if code(&secret, now()) == "000000" {
        "111111"
    } else {
        "000000"
    };

    let (status, _) = verify(&admin, &challenge, wrong).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        admin.last_audit("auth.totp.failed").await.outcome,
        "failure"
    );
}

#[tokio::test]
async fn existing_totp_cannot_be_replaced_with_a_password_only() {
    let admin = TestAdmin::with_totp(true).await;
    let account = admin.account("alice@example.com", &[]).await;
    let login = admin.login_with(&account.email, PASSWORD).await;
    let setup_challenge = login.json["challenge"].as_str().unwrap().to_owned();
    let setup = admin
        .call(
            Method::POST,
            "/auth/totp/setup",
            None,
            Some(json!({ "challenge": setup_challenge })),
        )
        .await;
    let secret = Secret::try_from_base32(setup.json["secret"].as_str().unwrap())
        .unwrap()
        .as_bytes()
        .to_vec();
    admin
        .call(
            Method::POST,
            "/auth/totp/confirm",
            None,
            Some(json!({ "challenge": setup_challenge, "code": code(&secret, now()) })),
        )
        .await;

    // Le challenge de configuration est encore valide, mais le TOTP existe désormais.
    let again = admin
        .call(
            Method::POST,
            "/auth/totp/setup",
            None,
            Some(json!({ "challenge": setup_challenge })),
        )
        .await;

    assert_eq!(again.status, StatusCode::CONFLICT);
    assert_eq!(again.json["code"], "totp_already_enabled");
}

#[tokio::test]
async fn totp_secret_is_encrypted_at_rest() {
    let admin = TestAdmin::with_totp(true).await;
    let account = admin.account("alice@example.com", &[]).await;
    let secret = enroll_totp(&admin, &account.email).await;

    let stored: Vec<u8> = sqlx::query_scalar("SELECT totp_secret FROM admin_users WHERE id = $1")
        .bind(account.id)
        .fetch_one(&admin.pool)
        .await
        .unwrap();

    assert!(
        !stored
            .windows(secret.len())
            .any(|window| window == secret.as_slice())
    );
}

#[tokio::test]
async fn dev_mode_skips_totp_for_accounts_without_one() {
    let admin = TestAdmin::with_totp(false).await;
    let account = admin.account("alice@example.com", &[]).await;

    let login = admin.login_with(&account.email, PASSWORD).await;

    assert_eq!(login.json["status"], "authenticated");
    assert_eq!(
        admin.last_audit("auth.login.succeeded").await.details["mfa"],
        false
    );
}
