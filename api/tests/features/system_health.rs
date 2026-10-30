use axum::http::{Method, StatusCode};
use serde_json::json;

use crate::support::admin::TestAdmin;
use crate::support::http as support;
use crate::support::http::call;

#[tokio::test]
async fn health_reports_ok() {
    let response = support::get("/api/health").await;

    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.json, json!({ "status": "ok" }));
}

/// Le pool des routes publiques ne joint aucune base : la sonde le dit, sans attendre.
#[tokio::test]
async fn ready_reports_an_unreachable_database_and_a_stopped_listener() {
    let response = support::get("/api/ready").await;

    assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response.json,
        json!({ "status": "not_ready", "database": "unavailable", "daemon_listener": "down" })
    );
}

#[tokio::test]
async fn ready_needs_the_database_and_the_daemon_listener() {
    let admin = TestAdmin::start().await;
    let router = admin.router();
    let ready = || call(&router, Method::GET, "/api/ready", None, None, None);

    let response = ready().await;
    assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.json["database"], "ok");
    assert_eq!(response.json["daemon_listener"], "down");

    admin.readiness.listener_up();
    let response = ready().await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(
        response.json,
        json!({ "status": "ready", "database": "ok", "daemon_listener": "ok" })
    );

    admin.readiness.listener_down();
    let response = ready().await;
    assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.json["status"], "not_ready");
}
