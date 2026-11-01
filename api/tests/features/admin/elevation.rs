//! Élévation de session (ADR-0050) : les actions protégées exigent, après la permission, une
//! preuve récente (mot de passe, ou code si le TOTP est actif), propre à la session.

use axum::http::{Method, StatusCode};
use kuro_api::auth::totp;
use kuro_api::iam::domain::Permission;
use serde_json::json;
use totp_rs::Secret;

use crate::support::admin::{Account, PASSWORD, Session, TestAdmin};
use crate::support::http::TestResponse;

fn now() -> u64 {
    u64::try_from(time::OffsetDateTime::now_utc().unix_timestamp()).unwrap()
}

/// Action protégée : suppression d'un compte admin.
async fn delete_user(admin: &TestAdmin, session: &Session, target: &Account) -> TestResponse {
    admin
        .call(
            Method::DELETE,
            &format!("/users/{}", target.id),
            Some(&session.access_token),
            None,
        )
        .await
}

async fn elevate(admin: &TestAdmin, session: &Session, body: serde_json::Value) -> TestResponse {
    admin
        .call(
            Method::POST,
            "/me/elevate",
            Some(&session.access_token),
            Some(body),
        )
        .await
}

fn assert_elevation_required(response: &TestResponse, method: &str) {
    assert_eq!(response.status, StatusCode::FORBIDDEN, "{}", response.json);
    assert_eq!(response.json["code"], "elevation_required");
    assert_eq!(response.json["params"]["method"], method);
    assert_eq!(
        response.headers["www-authenticate"],
        r#"Bearer error="insufficient_user_authentication""#
    );
}

/// Active le TOTP du compte connecté ; retourne le secret et les codes de secours.
async fn enable_totp(admin: &TestAdmin, session: &Session) -> (Vec<u8>, Vec<String>) {
    let setup = admin
        .call(
            Method::POST,
            "/me/totp/setup",
            Some(&session.access_token),
            Some(json!({ "password": PASSWORD })),
        )
        .await;
    let secret = Secret::try_from_base32(setup.json["secret"].as_str().unwrap())
        .unwrap()
        .as_bytes()
        .to_vec();
    let confirm = admin
        .call(
            Method::POST,
            "/me/totp/confirm",
            Some(&session.access_token),
            Some(json!({ "code": totp::current_code(&secret, now()).unwrap() })),
        )
        .await;
    assert_eq!(confirm.status, StatusCode::OK, "{}", confirm.json);
    let codes = confirm.json["recovery_codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|code| code.as_str().unwrap().to_owned())
        .collect();
    (secret, codes)
}

#[tokio::test]
async fn a_protected_action_waits_for_the_password_without_totp() {
    let admin = TestAdmin::start().await;
    let root = admin.super_admin("root@example.com").await;
    let bob = admin.account("bob@example.com", &[]).await;
    let session = admin.login_unelevated(&root).await;

    let refused = delete_user(&admin, &session, &bob).await;
    assert_elevation_required(&refused, "password");
    let denied = admin.last_audit("access.denied").await;
    assert_eq!(denied.details["reason"], "elevation_required");
    assert_eq!(denied.details["permission"], "iam.users.delete");

    let wrong = elevate(&admin, &session, json!({ "password": "not the password" })).await;
    assert_eq!(
        wrong.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        wrong.json
    );
    assert_eq!(wrong.json["fields"][0]["field"], "password");
    assert_eq!(
        admin.last_audit("auth.login.failed").await.outcome,
        "failure"
    );

    let granted = elevate(&admin, &session, json!({ "password": PASSWORD })).await;
    assert_eq!(granted.status, StatusCode::OK, "{}", granted.json);
    assert_eq!(granted.json["expires_in"], 300);
    let event = admin.last_audit("auth.elevation.granted").await;
    assert_eq!(event.details["method"], "password");
    assert_eq!(event.actor_id, Some(root.id));

    let deleted = delete_user(&admin, &session, &bob).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.json);
}

#[tokio::test]
async fn the_permission_is_checked_before_the_elevation() {
    let admin = TestAdmin::start().await;
    let alice = admin
        .account("alice@example.com", &[Permission::IamUsersRead])
        .await;
    let bob = admin.account("bob@example.com", &[]).await;
    let session = admin.login_unelevated(&alice).await;

    let response = delete_user(&admin, &session, &bob).await;

    assert_eq!(response.status, StatusCode::FORBIDDEN);
    assert_eq!(response.json["code"], "forbidden");
}

#[tokio::test]
async fn an_account_with_totp_elevates_with_a_code_or_a_recovery_code() {
    let admin = TestAdmin::start().await;
    let root = admin.super_admin("root@example.com").await;
    let bob = admin.account("bob@example.com", &[]).await;
    let carol = admin.account("carol@example.com", &[]).await;
    let session = admin.login_unelevated(&root).await;
    let (secret, codes) = enable_totp(&admin, &session).await;

    assert_elevation_required(&delete_user(&admin, &session, &bob).await, "totp");
    // Le mot de passe seul ne suffit plus.
    let password_only = elevate(&admin, &session, json!({ "password": PASSWORD })).await;
    assert_eq!(password_only.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        password_only.json["fields"][0]["code"],
        "totp_code_required"
    );

    let code = totp::current_code(&secret, now() + 30).unwrap();
    let granted = elevate(&admin, &session, json!({ "code": code })).await;
    assert_eq!(granted.status, StatusCode::OK, "{}", granted.json);
    assert_eq!(
        admin.last_audit("auth.elevation.granted").await.details["method"],
        "totp"
    );
    assert_eq!(
        delete_user(&admin, &session, &bob).await.status,
        StatusCode::NO_CONTENT
    );

    // Élévation passée : un code de secours tient lieu de code.
    sqlx::query("UPDATE admin_session_elevations SET elevated_until = now() - interval '1 second'")
        .execute(&admin.pool)
        .await
        .unwrap();
    assert_elevation_required(&delete_user(&admin, &session, &carol).await, "totp");
    let granted = elevate(&admin, &session, json!({ "code": codes[0] })).await;
    assert_eq!(granted.status, StatusCode::OK, "{}", granted.json);
    assert_eq!(
        admin.last_audit("auth.elevation.granted").await.details["method"],
        "recovery_code"
    );
    assert_eq!(
        admin
            .last_audit("auth.totp.recovery_code_used")
            .await
            .details["remaining"],
        9
    );
    assert_eq!(
        delete_user(&admin, &session, &carol).await.status,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn an_elevation_belongs_to_its_session_and_expires() {
    let admin = TestAdmin::start().await;
    let root = admin.super_admin("root@example.com").await;
    let bob = admin.account("bob@example.com", &[]).await;
    let carol = admin.account("carol@example.com", &[]).await;
    let elevated = admin.login(&root).await;
    let other = admin.login_unelevated(&root).await;

    // L'autre session du même compte n'en profite pas.
    assert_elevation_required(&delete_user(&admin, &other, &bob).await, "password");
    assert_eq!(
        delete_user(&admin, &elevated, &bob).await.status,
        StatusCode::NO_CONTENT
    );

    sqlx::query("UPDATE admin_session_elevations SET elevated_until = now() - interval '1 second'")
        .execute(&admin.pool)
        .await
        .unwrap();
    assert_elevation_required(&delete_user(&admin, &elevated, &carol).await, "password");
}
