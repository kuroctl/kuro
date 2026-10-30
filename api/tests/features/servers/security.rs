//! Un daemon n'agit que sur ses serveurs ; les secrets ne sortent jamais de l'API.

use axum::http::{Method, StatusCode};
use kuro_api::iam::domain::Permission;
use serde_json::json;

use crate::support::admin::TestAdmin;
use crate::support::platform::Platform;

#[tokio::test]
async fn a_node_cannot_change_the_state_of_another_nodes_server() {
    let platform = Platform::start().await;
    let mut owner = platform.online_node("node-1", "25565").await;
    let mut intruder = platform.online_node("node-2", "25566").await;
    let (server, _) = platform.deploy_minecraft(&mut owner, json!({})).await;
    let id = server["id"].as_str().unwrap().to_owned();
    owner.report(&id, "offline").await;
    platform.wait_for_state(&id, "offline").await;

    intruder.report(&id, "running").await;
    intruder.report(&id, "deleted").await;
    // Un message du nœud légitime ensuite : s'il est traité, ceux de l'intrus l'ont été avant.
    owner.report(&id, "starting").await;
    platform.wait_for_state(&id, "starting").await;

    let server = platform
        .call(Method::GET, &format!("/servers/{id}"), None)
        .await;
    assert_eq!(server.status, StatusCode::OK, "deleted by a foreign node");
}

#[tokio::test]
async fn creating_a_server_requires_its_permission() {
    let admin = TestAdmin::start().await;
    let account = admin
        .account("alice@example.com", &[Permission::ServersRead])
        .await;
    let session = admin.login(&account).await;

    let response = admin
        .call(
            Method::POST,
            "/servers",
            Some(&session.access_token),
            Some(json!({
                "name": "x", "node_id": uuid::Uuid::now_v7(), "prefab_id": "minecraft-java",
                "memory_mb": 1024, "cpu_percent": 100, "disk_mb": 1024,
            })),
        )
        .await;

    assert_eq!(response.status, StatusCode::FORBIDDEN);
    assert_eq!(response.json["params"]["permission"], "servers.create");
}

#[tokio::test]
async fn power_and_deletion_require_their_permissions() {
    let platform = Platform::start().await;
    let account = platform
        .admin
        .account("bob@example.com", &[Permission::ServersRead])
        .await;
    let token = platform.admin.login(&account).await.access_token;
    let id = uuid::Uuid::now_v7();

    let power = platform
        .admin
        .call(
            Method::POST,
            &format!("/servers/{id}/power"),
            Some(&token),
            Some(json!({ "action": "kill" })),
        )
        .await;
    let delete = platform
        .admin
        .call(
            Method::DELETE,
            &format!("/servers/{id}"),
            Some(&token),
            None,
        )
        .await;

    assert_eq!(power.json["params"]["permission"], "servers.power");
    assert_eq!(delete.json["params"]["permission"], "servers.delete");
}
