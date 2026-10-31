//! Première mise en route (ADR-0045) : état public, premier compte avec le jeton de l'hôte,
//! refus sans jeton ou après coup, concurrence, nom du projet, audit.

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

use crate::support::admin::TestAdmin;
use crate::support::http::PANEL_ORIGIN;

async fn status(admin: &TestAdmin) -> Value {
    let response = admin.raw(Method::GET, "/api/setup", &[], None).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    response.json
}

async fn setup(admin: &TestAdmin, body: Value) -> (StatusCode, Value) {
    let response = admin
        .raw(
            Method::POST,
            "/api/setup",
            &[("origin", PANEL_ORIGIN)],
            Some(("application/json", body.to_string())),
        )
        .await;
    (response.status, response.json)
}

fn request(token: &str) -> Value {
    json!({
        "token": token, "email": "owner@example.com", "display_name": "Owner",
        "password": "a long enough password", "project_name": "  Mon hébergement  ",
    })
}

/// Jeton écrit sur l'hôte au démarrage, lu comme le ferait l'admin.
async fn host_token(admin: &TestAdmin) -> String {
    let path = admin
        .setup
        .ensure_token()
        .await
        .unwrap()
        .unwrap()
        .to_owned();
    std::fs::read_to_string(path).unwrap().trim().to_owned()
}

#[tokio::test]
async fn a_fresh_api_needs_setup_until_the_first_account_is_created_with_the_host_token() {
    let admin = TestAdmin::start().await;
    assert_eq!(
        status(&admin).await,
        json!({ "needed": true, "project_name": null })
    );
    let token = host_token(&admin).await;
    assert!(token.starts_with("kuro_setup_"), "{token}");

    // Sans jeton, ou avec un autre : refusé, et rien n'est créé.
    let (refused, body) = setup(&admin, request("kuro_setup_wrong")).await;
    assert_eq!(refused, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "invalid_token");
    assert_eq!(status(&admin).await["needed"], true);

    let (created, body) = setup(&admin, request(&token)).await;
    assert_eq!(created, StatusCode::CREATED, "{body}");
    assert_eq!(body["email"], "owner@example.com");
    assert_eq!(
        status(&admin).await,
        json!({ "needed": false, "project_name": "Mon hébergement" })
    );
    // Le jeton a servi : le fichier est retiré, le compte est super admin et se connecte.
    assert!(!admin.setup.token_file.exists());
    let session = admin
        .call(
            Method::POST,
            "/auth/login",
            None,
            Some(json!({ "email": "owner@example.com", "password": "a long enough password" })),
        )
        .await;
    assert_eq!(session.status, StatusCode::OK, "{}", session.json);
    let access = session.json["access_token"].as_str().unwrap().to_owned();
    let me = admin.call(Method::GET, "/me", Some(&access), None).await;
    assert_eq!(me.json["all_permissions"], true, "{}", me.json);
    // Audité : le compte, puis la mise en route, par un acteur anonyme, sans secret.
    let created = admin.last_audit("iam.user.created").await;
    assert_eq!(created.actor_kind, "anonymous");
    let done = admin.last_audit("system.setup_completed").await;
    assert_eq!(done.details["project_name"], "Mon hébergement");
    assert_eq!(done.target_id.as_deref(), body["user_id"].as_str());
    for row in admin.audit().await {
        let text = row.details.to_string();
        assert!(
            !text.contains("kuro_setup_") && !text.contains("enough password"),
            "{text}"
        );
    }

    // Pour toujours : un second appel, même avec un jeton, est refusé.
    let (again, body) = setup(&admin, request(&token)).await;
    assert_eq!(again, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "setup_done");
    assert!(admin.setup.ensure_token().await.unwrap().is_none());
}

#[tokio::test]
async fn setup_is_refused_once_an_account_exists_whatever_its_origin() {
    let admin = TestAdmin::start().await;
    admin.super_admin("root@example.com").await;
    assert_eq!(status(&admin).await["needed"], false);
    let (refused, body) = setup(&admin, request("kuro_setup_whatever")).await;
    assert_eq!(refused, StatusCode::CONFLICT, "{body}");
}

#[tokio::test]
async fn concurrent_setups_create_a_single_account() {
    let admin = TestAdmin::start().await;
    let token = host_token(&admin).await;
    let first = setup(&admin, request(&token));
    let second = setup(
        &admin,
        json!({
            "token": token, "email": "other@example.com", "display_name": "Other",
            "password": "a long enough password",
        }),
    );
    let ((status_a, body_a), (status_b, body_b)) = tokio::join!(first, second);
    let mut statuses = [status_a, status_b];
    statuses.sort();
    assert_eq!(
        statuses,
        [StatusCode::CREATED, StatusCode::CONFLICT],
        "{body_a} / {body_b}"
    );
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM admin_users")
        .fetch_one(&admin.pool)
        .await
        .unwrap();
    assert_eq!(users, 1);
}

#[tokio::test]
async fn setup_fields_follow_the_admin_account_rules() {
    let admin = TestAdmin::start().await;
    let token = host_token(&admin).await;
    for (patch, field) in [
        (json!({ "email": "not-an-email" }), "email"),
        (json!({ "password": "short" }), "password"),
        (json!({ "display_name": " " }), "display_name"),
        (json!({ "project_name": "x".repeat(81) }), "project_name"),
    ] {
        let mut body = request(&token);
        for (key, value) in patch.as_object().unwrap() {
            body[key] = value.clone();
        }
        let (status, json) = setup(&admin, body).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{field}: {json}");
        assert!(
            json["fields"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["field"] == field),
            "{field}: {json}"
        );
    }
    assert_eq!(status(&admin).await["needed"], true);
}

#[tokio::test]
async fn the_project_name_is_a_setting_shown_to_everyone() {
    let admin = TestAdmin::start().await;
    let root = admin.super_admin("root@example.com").await;
    let token = admin.login(&root).await.access_token;
    let updated = admin
        .call(
            Method::PATCH,
            "/settings",
            Some(&token),
            Some(json!({ "project_name": "Studio Nuage" })),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.json);
    assert_eq!(updated.json["project_name"], "Studio Nuage");
    assert_eq!(status(&admin).await["project_name"], "Studio Nuage");
    let event = admin.last_audit("system.settings_updated").await;
    assert_eq!(
        event.details["changed"]["project_name"]["to"],
        "Studio Nuage"
    );

    let cleared = admin
        .call(
            Method::PATCH,
            "/settings",
            Some(&token),
            Some(json!({ "project_name": null })),
        )
        .await;
    assert_eq!(cleared.status, StatusCode::OK, "{}", cleared.json);
    assert!(cleared.json["project_name"].is_null());
    let refused = admin
        .call(
            Method::PATCH,
            "/settings",
            Some(&token),
            Some(json!({ "project_name": "x".repeat(81) })),
        )
        .await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
}
