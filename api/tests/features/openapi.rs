use axum::http::StatusCode;

use crate::support::http as support;

#[tokio::test]
async fn openapi_document_is_served_as_json() {
    let response = support::get("/api/openapi.json").await;

    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.content_type.as_deref(), Some("application/json"));
    assert!(response.json["openapi"].is_string());
}

#[tokio::test]
async fn openapi_document_includes_domain_routes() {
    let response = support::get("/api/openapi.json").await;

    for path in [
        "/api/health",
        "/api/ready",
        "/api/admin/v1/auth/login",
        "/api/admin/v1/users",
        "/api/admin/v1/audit",
        "/api/enroll",
    ] {
        assert!(response.json["paths"][path].is_object(), "{path} missing");
    }
    assert!(response.json["components"]["securitySchemes"]["bearer"].is_object());
}
