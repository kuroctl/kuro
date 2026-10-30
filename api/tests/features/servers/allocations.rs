//! Ports des nœuds : plages, validation, suppression d'un port libre seulement.

use axum::http::{Method, StatusCode};
use kuro_api::iam::domain::Permission;
use serde_json::json;

use crate::support::admin::TestAdmin;
use crate::support::platform::Platform;

#[tokio::test]
async fn ranges_and_lists_create_missing_ports_only() {
    let platform = Platform::start().await;
    let node = platform.api.enroll_node("node-1").await;
    let path = format!("/nodes/{}/allocations", node.id);

    let first = platform
        .call(Method::POST, &path, Some(json!({ "ports": "25565-25567" })))
        .await;
    let second = platform
        .call(
            Method::POST,
            &path,
            Some(json!({ "ip": "127.0.0.1", "ports": "25567, 25570" })),
        )
        .await;

    assert_eq!(first.status, StatusCode::CREATED);
    assert_eq!(first.json.as_array().unwrap().len(), 3);
    assert_eq!(second.json.as_array().unwrap().len(), 5);
    let list = platform.call(Method::GET, &path, None).await;
    let ports: Vec<(String, i64)> = list
        .json
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            (
                a["ip"].as_str().unwrap().to_owned(),
                a["port"].as_i64().unwrap(),
            )
        })
        .collect();
    assert!(ports.contains(&("0.0.0.0".to_owned(), 25566)));
    assert!(ports.contains(&("127.0.0.1".to_owned(), 25570)));
    let event = platform.admin.last_audit("allocation.created").await;
    assert_eq!(event.details["created"], 2);
}

#[tokio::test]
async fn invalid_ranges_are_reported_on_their_field() {
    let platform = Platform::start().await;
    let node = platform.api.enroll_node("node-1").await;
    let path = format!("/nodes/{}/allocations", node.id);

    for (body, field, code) in [
        (json!({ "ports": "80" }), "ports", "invalid_port_range"),
        (
            json!({ "ports": "30000-29000" }),
            "ports",
            "invalid_port_range",
        ),
        (
            json!({ "ports": "20000-40000" }),
            "ports",
            "invalid_port_range",
        ),
        (json!({ "ports": "abc" }), "ports", "invalid_port_range"),
        (
            json!({ "ip": "not-an-ip", "ports": "25565" }),
            "ip",
            "invalid_ip",
        ),
    ] {
        let response = platform.call(Method::POST, &path, Some(body.clone())).await;

        assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(response.json["fields"][0]["field"], field, "{body}");
        assert_eq!(response.json["fields"][0]["code"], code, "{body}");
    }
}

#[tokio::test]
async fn an_assigned_port_cannot_be_deleted() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25566").await;
    let (server, _) = platform.deploy_minecraft(&mut daemon, json!({})).await;
    let path = format!("/nodes/{}/allocations", daemon.node.id);
    let list = platform.call(Method::GET, &path, None).await;
    let allocations = list.json.as_array().unwrap();
    let assigned = allocations
        .iter()
        .find(|a| a["server_id"] == server["id"])
        .unwrap();
    let free = allocations
        .iter()
        .find(|a| a["server_id"].is_null())
        .unwrap();

    let refused = platform
        .call(
            Method::DELETE,
            &format!("{path}/{}", assigned["id"].as_str().unwrap()),
            None,
        )
        .await;
    let deleted = platform
        .call(
            Method::DELETE,
            &format!("{path}/{}", free["id"].as_str().unwrap()),
            None,
        )
        .await;

    assert_eq!(refused.status, StatusCode::CONFLICT);
    assert_eq!(refused.json["code"], "allocation_in_use");
    assert_eq!(assigned["server_name"], "Survie");
    assert_eq!(assigned["port_name"], "game");
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn managing_ports_requires_its_permission() {
    let admin = TestAdmin::start().await;
    let account = admin
        .account("alice@example.com", &[Permission::NodesAllocationsRead])
        .await;
    let session = admin.login(&account).await;

    let response = admin
        .call(
            axum::http::Method::POST,
            &format!("/nodes/{}/allocations", uuid::Uuid::now_v7()),
            Some(&session.access_token),
            Some(json!({ "ports": "25565" })),
        )
        .await;

    assert_eq!(response.status, StatusCode::FORBIDDEN);
    assert_eq!(
        response.json["params"]["permission"],
        "nodes.allocations.manage"
    );
}
