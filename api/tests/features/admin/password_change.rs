//! Mot de passe choisi par un autre admin : à remplacer à la connexion, avant toute session
//! et avant le TOTP. Choisi depuis l'hôte (CLI), il reste celui de l'opérateur.

use axum::http::{Method, StatusCode};
use kuro_api::audit::domain::AuditContext;
use kuro_api::iam::application::users;
use kuro_api::iam::domain::Authority;
use serde_json::{Value, json};

use crate::support::admin::{Account, PASSWORD, REFRESH_COOKIE, TestAdmin};
use crate::support::http::TestResponse;

const TEMPORARY: &str = "temporary password 1";
const CHOSEN: &str = "my very own password";

/// Compte créé par un super-admin dans le panel, avec le mot de passe `TEMPORARY`.
async fn created_by_admin(admin: &TestAdmin) -> (Account, Value) {
    let root = admin.super_admin("root@example.com").await;
    let session = admin.login(&root).await;
    let created = admin
        .call(
            Method::POST,
            "/users",
            Some(&session.access_token),
            Some(
                json!({ "email": "bob@example.com", "display_name": "Bob", "password": TEMPORARY }),
            ),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json);
    let id = created.json["id"].as_str().unwrap().parse().unwrap();
    (
        Account {
            id,
            email: "bob@example.com".to_owned(),
        },
        created.json,
    )
}

async fn change_challenge(admin: &TestAdmin, email: &str, password: &str) -> String {
    let login = admin.login_with(email, password).await;
    assert_eq!(login.status, StatusCode::OK, "{}", login.json);
    assert_eq!(
        login.json["status"], "password_change_required",
        "{}",
        login.json
    );
    assert!(login.cookie(REFRESH_COOKIE).is_none(), "no session yet");
    login.json["challenge"].as_str().unwrap().to_owned()
}

async fn change(admin: &TestAdmin, challenge: &str, new_password: &str) -> TestResponse {
    admin
        .call(
            Method::POST,
            "/auth/password",
            None,
            Some(json!({ "challenge": challenge, "new_password": new_password })),
        )
        .await
}

#[tokio::test]
async fn an_account_created_by_an_admin_replaces_its_password_before_any_session() {
    let admin = TestAdmin::start().await;
    let (bob, created) = created_by_admin(&admin).await;
    assert_eq!(created["password_change_required"], true);

    let challenge = change_challenge(&admin, &bob.email, TEMPORARY).await;
    let weak = change(&admin, &challenge, "short").await;
    assert_eq!(weak.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(weak.json["fields"][0]["field"], "new_password");
    assert_eq!(weak.json["code"], "weak_password");
    let same = change(&admin, &challenge, TEMPORARY).await;
    assert_eq!(same.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(same.json["code"], "password_unchanged");

    let changed = change(&admin, &challenge, CHOSEN).await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.json);
    assert_eq!(changed.json["status"], "authenticated");
    assert!(changed.cookie(REFRESH_COOKIE).is_some());
    let token = changed.json["access_token"].as_str().unwrap();
    let me = admin.call(Method::GET, "/me", Some(token), None).await;
    assert_eq!(me.status, StatusCode::OK);

    let event = admin.last_audit("iam.self.password_changed").await;
    assert_eq!(event.actor_id, Some(bob.id));
    assert_eq!(event.details["forced"], true);
    admin.last_audit("auth.login.succeeded").await;

    // Le challenge ne sert qu'une fois ; ensuite, le nouveau mot de passe ouvre une session.
    let replayed = change(&admin, &challenge, "yet another password").await;
    assert_eq!(replayed.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        admin.login_with(&bob.email, TEMPORARY).await.status,
        StatusCode::UNAUTHORIZED
    );
    let login = admin.login_with(&bob.email, CHOSEN).await;
    assert_eq!(login.json["status"], "authenticated", "{}", login.json);
}

#[tokio::test]
async fn the_password_comes_before_the_totp_setup() {
    let admin = TestAdmin::with_totp(true).await;
    // Un super-admin sans TOTP ne peut pas créer de compte ici : l'obligation est posée en base,
    // comme le ferait une création depuis le panel (testée ci-dessus).
    let bob = admin.account("bob@example.com", &[]).await;
    sqlx::query("UPDATE admin_users SET password_change_required = true WHERE id = $1")
        .bind(bob.id)
        .execute(&admin.pool)
        .await
        .unwrap();

    let challenge = change_challenge(&admin, &bob.email, PASSWORD).await;
    // Le challenge du mot de passe ne vaut pas pour le TOTP.
    let setup = admin
        .call(
            Method::POST,
            "/auth/totp/setup",
            None,
            Some(json!({ "challenge": challenge })),
        )
        .await;
    assert_eq!(setup.status, StatusCode::UNAUTHORIZED);

    let changed = change(&admin, &challenge, CHOSEN).await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.json);
    assert_eq!(changed.json["status"], "totp_setup_required");
    assert!(
        changed.cookie(REFRESH_COOKIE).is_none(),
        "no session before the TOTP"
    );
    // Le changement est acquis même si le TOTP n'est pas encore configuré.
    admin.last_audit("iam.self.password_changed").await;
    let again = admin.login_with(&bob.email, CHOSEN).await;
    assert_eq!(
        again.json["status"], "totp_setup_required",
        "{}",
        again.json
    );
}

#[tokio::test]
async fn a_reset_by_an_admin_must_be_replaced_but_not_one_from_the_host() {
    let admin = TestAdmin::start().await;
    let root = admin.super_admin("root@example.com").await;
    let session = admin.login(&root).await;
    let target = admin.account("target@example.com", &[]).await;

    let reset = admin
        .call(
            Method::PUT,
            &format!("/users/{}/password", target.id),
            Some(&session.access_token),
            Some(json!({ "password": TEMPORARY })),
        )
        .await;
    assert_eq!(reset.status, StatusCode::NO_CONTENT);
    let user = admin
        .call(
            Method::GET,
            &format!("/users/{}", target.id),
            Some(&session.access_token),
            None,
        )
        .await;
    assert_eq!(user.json["password_change_required"], true);
    change_challenge(&admin, &target.email, TEMPORARY).await;

    // Secours depuis l'hôte : l'opérateur tape son propre mot de passe, l'obligation est levée.
    users::reset_password(
        &admin.iam,
        &Authority::Host,
        &AuditContext::cli("test"),
        target.id,
        CHOSEN,
    )
    .await
    .unwrap();
    let login = admin.login_with(&target.email, CHOSEN).await;
    assert_eq!(login.json["status"], "authenticated", "{}", login.json);
}
