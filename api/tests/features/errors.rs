//! Contrat d'erreur consommé par le panel : RFC 9457 + `code`, `params`, `fields`.

use axum::http::{Method, StatusCode};
use kuro_api::error_code::ErrorCode;
use serde_json::json;

use crate::support::admin::TestAdmin;
use crate::support::http::get;

#[tokio::test]
async fn errors_are_problem_json_with_a_stable_code_and_a_request_id() {
    let admin = TestAdmin::start().await;

    let response = admin.call(Method::GET, "/me", None, None).await;

    assert_eq!(response.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        response.content_type.as_deref(),
        Some("application/problem+json")
    );
    assert_eq!(response.json["type"], "about:blank");
    assert_eq!(response.json["status"], 401);
    assert_eq!(response.json["code"], "unauthorized");
    assert_eq!(response.json["title"], "Authentication required");
    assert!(response.headers.get("x-request-id").is_some());
}

#[tokio::test]
async fn validation_errors_name_the_offending_field() {
    let admin = TestAdmin::start().await;
    let root = admin.super_admin("root@example.com").await;
    let session = admin.login(&root).await;
    let token = Some(session.access_token.as_str());

    let user = admin
        .call(
            Method::POST,
            "/users",
            token,
            Some(json!({ "email": "nope", "display_name": "X", "password": "a long enough password" })),
        )
        .await;
    let password = admin
        .call(
            Method::PUT,
            "/me/password",
            token,
            Some(json!({ "current_password": "whatever", "new_password": "short" })),
        )
        .await;

    assert_eq!(user.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(user.json["code"], "invalid_email");
    assert_eq!(
        user.json["fields"],
        json!([{ "field": "email", "code": "invalid_email" }])
    );
    assert_eq!(password.json["fields"][0]["field"], "new_password");
    assert_eq!(password.json["fields"][0]["code"], "weak_password");
}

#[tokio::test]
async fn error_parameters_feed_the_translated_message() {
    let admin = TestAdmin::start().await;
    let root = admin.super_admin("root@example.com").await;
    let session = admin.login(&root).await;

    let response = admin
        .call(
            Method::POST,
            "/roles",
            Some(&session.access_token),
            Some(json!({ "name": "x", "permissions": ["servers.transfer"] })),
        )
        .await;

    assert_eq!(response.json["code"], "permission_not_assignable");
    assert_eq!(
        response.json["params"],
        json!({ "permission": "servers.transfer" })
    );
    assert_eq!(response.json["fields"][0]["field"], "permissions");
}

#[tokio::test]
async fn malformed_requests_get_the_same_contract() {
    let admin = TestAdmin::start().await;
    let root = admin.super_admin("root@example.com").await;
    let session = admin.login(&root).await;
    let token = Some(session.access_token.as_str());

    let missing_body = admin.call(Method::POST, "/auth/login", None, None).await;
    let wrong_shape = admin
        .call(
            Method::POST,
            "/auth/login",
            None,
            Some(json!({ "email": 42 })),
        )
        .await;
    let bad_uuid = admin
        .call(Method::GET, "/users/not-a-uuid", token, None)
        .await;
    let unknown_route = admin
        .call(Method::GET, "/does-not-exist", token, None)
        .await;

    assert_eq!(missing_body.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(missing_body.json["code"], "invalid_body");
    assert_eq!(wrong_shape.json["code"], "invalid_body");
    assert!(wrong_shape.json["params"]["reason"].is_string());
    assert_eq!(bad_uuid.status, StatusCode::BAD_REQUEST);
    assert_eq!(bad_uuid.json["code"], "invalid_parameter");
    assert_eq!(unknown_route.status, StatusCode::NOT_FOUND);
    assert_eq!(
        unknown_route.content_type.as_deref(),
        Some("application/problem+json")
    );
    assert_eq!(unknown_route.json["code"], "not_found");
}

#[tokio::test]
async fn every_error_code_is_published_in_the_openapi_document() {
    let response = get("/api/openapi.json").await;

    let published: Vec<&str> = response.json["components"]["schemas"]["ErrorCode"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|code| code.as_str().unwrap())
        .collect();
    let catalog: Vec<&str> = ErrorCode::ALL.iter().map(|code| code.as_str()).collect();

    assert_eq!(published, catalog);
    assert!(response.json["components"]["schemas"]["Problem"].is_object());
}

#[tokio::test]
async fn every_permission_key_is_published_in_the_openapi_document() {
    let response = get("/api/openapi.json").await;

    let published: Vec<&str> = response.json["components"]["schemas"]["PermissionKey"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|key| key.as_str().unwrap())
        .collect();
    let catalog: Vec<&str> = kuro_api::iam::domain::Permission::ALL
        .iter()
        .map(|permission| permission.key())
        .collect();

    assert_eq!(published, catalog);
}
