//! TOTP géré depuis son compte (activation, remplacement, désactivation) et codes de secours.

use axum::http::{Method, StatusCode};
use kuro_api::auth::totp;
use serde_json::{Value, json};
use totp_rs::Secret;

use crate::support::admin::{PASSWORD, REFRESH_COOKIE, Session, TestAdmin};
use crate::support::http::TestResponse;

fn now() -> u64 {
    u64::try_from(time::OffsetDateTime::now_utc().unix_timestamp()).unwrap()
}

fn code(secret: &[u8], at: u64) -> String {
    totp::current_code(secret, at).unwrap()
}

fn secret_of(setup: &TestResponse) -> Vec<u8> {
    Secret::try_from_base32(setup.json["secret"].as_str().unwrap())
        .unwrap()
        .as_bytes()
        .to_vec()
}

fn codes_of(response: &TestResponse) -> Vec<String> {
    response.json["recovery_codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|code| code.as_str().unwrap().to_owned())
        .collect()
}

async fn me(admin: &TestAdmin, session: &Session) -> Value {
    let response = admin
        .call(Method::GET, "/me", Some(&session.access_token), None)
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    response.json
}

async fn post(admin: &TestAdmin, session: &Session, path: &str, body: Value) -> TestResponse {
    admin
        .call(Method::POST, path, Some(&session.access_token), Some(body))
        .await
}

/// Active le TOTP depuis le compte (aucun TOTP auparavant) ; retourne le secret et les codes.
async fn enable(admin: &TestAdmin, session: &Session) -> (Vec<u8>, Vec<String>) {
    let setup = post(
        admin,
        session,
        "/me/totp/setup",
        json!({ "password": PASSWORD }),
    )
    .await;
    assert_eq!(setup.status, StatusCode::OK, "{}", setup.json);
    let secret = secret_of(&setup);
    let confirm = post(
        admin,
        session,
        "/me/totp/confirm",
        json!({ "code": code(&secret, now()) }),
    )
    .await;
    assert_eq!(confirm.status, StatusCode::OK, "{}", confirm.json);
    (secret, codes_of(&confirm))
}

/// Connexion jusqu'au défi TOTP, puis `code` (TOTP ou code de secours).
async fn login_with_code(admin: &TestAdmin, email: &str, code: &str) -> TestResponse {
    let login = admin.login_with(email, PASSWORD).await;
    assert_eq!(login.json["status"], "totp_required", "{}", login.json);
    admin
        .call(
            Method::POST,
            "/auth/totp/verify",
            None,
            Some(json!({ "challenge": login.json["challenge"], "code": code })),
        )
        .await
}

#[tokio::test]
async fn account_without_totp_can_enable_it_and_gets_recovery_codes() {
    let admin = TestAdmin::with_totp(false).await;
    let account = admin.account("alice@example.com", &[]).await;
    let session = admin.login(&account).await;
    let before = me(&admin, &session).await;
    assert_eq!(before["user"]["totp_enabled"], false);
    assert_eq!(before["totp_required"], false);

    let (_, codes) = enable(&admin, &session).await;

    assert_eq!(codes.len(), 10);
    assert!(codes.iter().all(|code| code.len() == 14));
    let started = admin.last_audit("auth.totp.setup_started").await;
    assert_eq!(started.details["replace"], false);
    assert_eq!(started.actor_id, Some(account.id));
    admin.last_audit("auth.totp.enabled").await;
    let after = me(&admin, &session).await;
    assert_eq!(after["user"]["totp_enabled"], true);
    assert_eq!(after["recovery_codes_remaining"], 10);
    let login = admin.login_with(&account.email, PASSWORD).await;
    assert_eq!(login.json["status"], "totp_required");
}

#[tokio::test]
async fn wrong_password_is_refused_on_the_field_and_audited() {
    let admin = TestAdmin::with_totp(false).await;
    let account = admin.account("alice@example.com", &[]).await;
    let session = admin.login(&account).await;

    let setup = post(
        &admin,
        &session,
        "/me/totp/setup",
        json!({ "password": "not the password" }),
    )
    .await;

    // Jamais un 401 : le panel le prendrait pour un jeton expiré et rejouerait la requête.
    assert_eq!(
        setup.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        setup.json
    );
    assert_eq!(
        setup.json["fields"][0]["field"], "password",
        "{}",
        setup.json
    );
    assert_eq!(
        admin.last_audit("auth.login.failed").await.outcome,
        "failure"
    );
}

#[tokio::test]
async fn wrong_confirmation_code_is_refused_and_audited() {
    let admin = TestAdmin::with_totp(false).await;
    let account = admin.account("alice@example.com", &[]).await;
    let session = admin.login(&account).await;
    let setup = post(
        &admin,
        &session,
        "/me/totp/setup",
        json!({ "password": PASSWORD }),
    )
    .await;
    let secret = secret_of(&setup);
    let wrong = if code(&secret, now()) == "000000" {
        "111111"
    } else {
        "000000"
    };

    let confirm = post(
        &admin,
        &session,
        "/me/totp/confirm",
        json!({ "code": wrong }),
    )
    .await;

    assert_eq!(
        confirm.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        confirm.json
    );
    assert_eq!(confirm.json["fields"][0]["code"], "invalid_code");
    assert_eq!(
        admin.last_audit("auth.totp.failed").await.outcome,
        "failure"
    );
    assert_eq!(me(&admin, &session).await["user"]["totp_enabled"], false);
}

#[tokio::test]
async fn replacing_totp_requires_the_current_code_and_closes_other_sessions() {
    let admin = TestAdmin::with_totp(false).await;
    let account = admin.account("alice@example.com", &[]).await;
    let session = admin.login(&account).await;
    let other = admin.login(&account).await;
    let (secret, _) = enable(&admin, &session).await;

    let without = post(
        &admin,
        &session,
        "/me/totp/setup",
        json!({ "password": PASSWORD }),
    )
    .await;
    assert_eq!(without.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(without.json["fields"][0]["code"], "totp_code_required");

    let setup = post(
        &admin,
        &session,
        "/me/totp/setup",
        json!({ "password": PASSWORD, "code": code(&secret, now() + 30) }),
    )
    .await;
    assert_eq!(setup.status, StatusCode::OK, "{}", setup.json);
    assert_eq!(
        admin.last_audit("auth.totp.setup_started").await.details["replace"],
        true
    );
    let new_secret = secret_of(&setup);
    let confirm = post(
        &admin,
        &session,
        "/me/totp/confirm",
        json!({ "code": code(&new_secret, now()) }),
    )
    .await;

    assert_eq!(confirm.status, StatusCode::OK, "{}", confirm.json);
    let replaced = admin.last_audit("auth.totp.replaced").await;
    assert_eq!(replaced.details["revoked_sessions"], 1);
    let closed = admin
        .call(Method::GET, "/me", Some(&other.access_token), None)
        .await;
    assert_eq!(closed.status, StatusCode::UNAUTHORIZED);
    me(&admin, &session).await;
    // L'ancien secret ne vaut plus rien, le nouveau ouvre une session.
    let old = login_with_code(&admin, &account.email, &code(&secret, now() + 30)).await;
    assert_eq!(old.status, StatusCode::UNAUTHORIZED);
    let new = login_with_code(&admin, &account.email, &code(&new_secret, now() + 30)).await;
    assert_eq!(new.status, StatusCode::OK, "{}", new.json);
}

#[tokio::test]
async fn recovery_code_opens_a_session_once() {
    let admin = TestAdmin::with_totp(false).await;
    let account = admin.account("alice@example.com", &[]).await;
    let session = admin.login(&account).await;
    let (_, codes) = enable(&admin, &session).await;

    // Saisie tolérante : majuscules, sans tirets.
    let typed = codes[0].replace('-', "").to_uppercase();
    let first = login_with_code(&admin, &account.email, &typed).await;
    let again = login_with_code(&admin, &account.email, &codes[0]).await;

    assert_eq!(first.status, StatusCode::OK, "{}", first.json);
    assert!(first.cookie(REFRESH_COOKIE).is_some());
    assert_eq!(
        admin.last_audit("auth.login.succeeded").await.details["mfa"],
        "recovery_code"
    );
    assert_eq!(
        admin
            .last_audit("auth.totp.recovery_code_used")
            .await
            .details["remaining"],
        9
    );
    assert_eq!(again.status, StatusCode::UNAUTHORIZED);
    assert_eq!(again.json["code"], "invalid_code");
    assert_eq!(
        admin.last_audit("auth.totp.failed").await.outcome,
        "failure"
    );
}

#[tokio::test]
async fn recovery_code_stands_in_for_a_lost_phone_when_replacing_totp() {
    let admin = TestAdmin::with_totp(false).await;
    let account = admin.account("alice@example.com", &[]).await;
    let session = admin.login(&account).await;
    let (_, codes) = enable(&admin, &session).await;

    let setup = post(
        &admin,
        &session,
        "/me/totp/setup",
        json!({ "password": PASSWORD, "code": codes[3] }),
    )
    .await;

    assert_eq!(setup.status, StatusCode::OK, "{}", setup.json);
    assert_eq!(
        admin
            .last_audit("auth.totp.recovery_code_used")
            .await
            .details["remaining"],
        9
    );
}

#[tokio::test]
async fn regenerating_recovery_codes_invalidates_the_previous_ones() {
    let admin = TestAdmin::with_totp(false).await;
    let account = admin.account("alice@example.com", &[]).await;
    let session = admin.login(&account).await;
    let (secret, old_codes) = enable(&admin, &session).await;

    let regenerated = post(
        &admin,
        &session,
        "/me/totp/recovery-codes",
        json!({ "password": PASSWORD, "code": code(&secret, now() + 30) }),
    )
    .await;

    assert_eq!(regenerated.status, StatusCode::OK, "{}", regenerated.json);
    admin
        .last_audit("auth.totp.recovery_codes_regenerated")
        .await;
    let new_codes = codes_of(&regenerated);
    assert_ne!(new_codes, old_codes);
    let old = login_with_code(&admin, &account.email, &old_codes[0]).await;
    assert_eq!(old.status, StatusCode::UNAUTHORIZED);
    let new = login_with_code(&admin, &account.email, &new_codes[0]).await;
    assert_eq!(new.status, StatusCode::OK, "{}", new.json);
}

#[tokio::test]
async fn regenerating_without_totp_is_a_conflict() {
    let admin = TestAdmin::with_totp(false).await;
    let account = admin.account("alice@example.com", &[]).await;
    let session = admin.login(&account).await;

    let response = post(
        &admin,
        &session,
        "/me/totp/recovery-codes",
        json!({ "password": PASSWORD }),
    )
    .await;

    assert_eq!(response.status, StatusCode::CONFLICT);
    assert_eq!(response.json["code"], "totp_not_enabled");
}

#[tokio::test]
async fn totp_can_be_disabled_when_the_platform_does_not_require_it() {
    let admin = TestAdmin::with_totp(false).await;
    let account = admin.account("alice@example.com", &[]).await;
    let session = admin.login(&account).await;
    let (secret, _) = enable(&admin, &session).await;

    let response = admin
        .call(
            Method::DELETE,
            "/me/totp",
            Some(&session.access_token),
            Some(json!({ "password": PASSWORD, "code": code(&secret, now() + 30) })),
        )
        .await;

    assert_eq!(response.status, StatusCode::NO_CONTENT, "{}", response.json);
    admin.last_audit("auth.totp.disabled").await;
    let profile = me(&admin, &session).await;
    assert_eq!(profile["user"]["totp_enabled"], false);
    assert_eq!(profile["recovery_codes_remaining"], 0);
    let login = admin.login_with(&account.email, PASSWORD).await;
    assert_eq!(login.json["status"], "authenticated");
}

#[tokio::test]
async fn totp_cannot_be_disabled_when_the_platform_requires_it() {
    let admin = TestAdmin::with_totp(true).await;
    let account = admin.account("alice@example.com", &[]).await;
    let login = admin.login_with(&account.email, PASSWORD).await;
    let challenge = login.json["challenge"].as_str().unwrap().to_owned();
    let setup = admin
        .call(
            Method::POST,
            "/auth/totp/setup",
            None,
            Some(json!({ "challenge": challenge })),
        )
        .await;
    let secret = secret_of(&setup);
    let confirm = admin
        .call(
            Method::POST,
            "/auth/totp/confirm",
            None,
            Some(json!({ "challenge": challenge, "code": code(&secret, now()) })),
        )
        .await;
    // La configuration à la connexion délivre aussi les codes de secours.
    assert_eq!(codes_of(&confirm).len(), 10);
    let token = confirm.json["access_token"].as_str().unwrap();
    let profile = admin.call(Method::GET, "/me", Some(token), None).await;
    assert_eq!(profile.json["totp_required"], true);
    assert_eq!(profile.json["recovery_codes_remaining"], 10);

    let response = admin
        .call(
            Method::DELETE,
            "/me/totp",
            Some(token),
            Some(json!({ "password": PASSWORD, "code": code(&secret, now() + 30) })),
        )
        .await;

    assert_eq!(response.status, StatusCode::FORBIDDEN);
    assert_eq!(response.json["code"], "totp_required_by_policy");
}

#[tokio::test]
async fn admin_reset_also_removes_recovery_codes() {
    let admin = TestAdmin::with_totp(false).await;
    let account = admin.account("alice@example.com", &[]).await;
    let session = admin.login(&account).await;
    enable(&admin, &session).await;
    let boss = admin.super_admin("boss@example.com").await;
    let boss_session = admin.login(&boss).await;

    let reset = admin
        .call(
            Method::DELETE,
            &format!("/users/{}/totp", account.id),
            Some(&boss_session.access_token),
            None,
        )
        .await;

    assert_eq!(reset.status, StatusCode::NO_CONTENT, "{}", reset.json);
    let left: i64 =
        sqlx::query_scalar("SELECT count(*) FROM admin_recovery_codes WHERE user_id = $1")
            .bind(account.id)
            .fetch_one(&admin.pool)
            .await
            .unwrap();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn recovery_codes_are_not_stored_in_clear() {
    let admin = TestAdmin::with_totp(false).await;
    let account = admin.account("alice@example.com", &[]).await;
    let session = admin.login(&account).await;
    let (_, codes) = enable(&admin, &session).await;

    let stored: Vec<Vec<u8>> =
        sqlx::query_scalar("SELECT code_hash FROM admin_recovery_codes WHERE user_id = $1")
            .bind(account.id)
            .fetch_all(&admin.pool)
            .await
            .unwrap();

    assert_eq!(stored.len(), 10);
    for code in &codes {
        let canonical = code.replace('-', "");
        assert!(stored.iter().all(|hash| {
            !hash
                .windows(canonical.len())
                .any(|window| window == canonical.as_bytes())
        }));
    }
}
