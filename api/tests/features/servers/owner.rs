//! Propriétaire d'un serveur (ADR-0051) : identifiant opaque posé à la création ou transféré,
//! format vérifié, filtre de la liste, audit du transfert, permission `servers.update`.

use axum::http::{Method, StatusCode};
use kuro_api::iam::domain::Permission;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::platform::{FakeDaemon, Platform};

const ALICE: &str = "cloud:6f1c2a9e-3b7d-4c8e-9a51-2d4f8b0c7e13";
const BOB: &str = "cloud:cus_8f2k1q";

async fn deploy(platform: &Platform, daemon: &mut FakeDaemon, name: &str, owner: Value) -> Value {
    let (response, _) = tokio::join!(
        platform.call(
            Method::POST,
            "/servers",
            Some(json!({
                "name": name,
                "node_id": daemon.node.id.as_uuid(),
                "prefab_id": "minecraft-java",
                "owner_ref": owner,
            })),
        ),
        daemon.accept(),
    );
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.json);
    response.json
}

fn id_of(server: &Value) -> &str {
    server["id"].as_str().unwrap()
}

#[tokio::test]
async fn creation_records_the_owner() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;

    let server = deploy(&platform, &mut daemon, "Survie", json!(ALICE)).await;

    assert_eq!(server["owner_ref"], ALICE);
    let audit = platform.admin.last_audit("server.created").await;
    assert_eq!(audit.details["owner_ref"], ALICE);
    let read = platform
        .call(Method::GET, &format!("/servers/{}", id_of(&server)), None)
        .await;
    assert_eq!(read.json["owner_ref"], ALICE);
}

#[tokio::test]
async fn a_server_without_owner_is_internal() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;

    let server = deploy(&platform, &mut daemon, "Interne", Value::Null).await;

    assert_eq!(server["owner_ref"], Value::Null);
}

#[tokio::test]
async fn creation_refuses_a_malformed_owner_on_its_field() {
    let platform = Platform::start().await;
    let daemon = platform.online_node("node-1", "25565-25570").await;

    for owner in ["alice", "Cloud:alice", "cloud:a b", "cloud:"] {
        let response = platform
            .call(
                Method::POST,
                "/servers",
                Some(json!({
                    "name": "Survie",
                    "node_id": daemon.node.id.as_uuid(),
                    "prefab_id": "minecraft-java",
                    "owner_ref": owner,
                })),
            )
            .await;

        assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY, "{owner}");
        assert_eq!(response.json["fields"][0]["field"], "owner_ref");
        assert_eq!(response.json["fields"][0]["code"], "invalid_owner_ref");
    }
    let listed = platform.call(Method::GET, "/servers", None).await;
    assert_eq!(listed.json, json!([]));
}

#[tokio::test]
async fn transfer_changes_the_owner_and_is_audited() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let server = deploy(&platform, &mut daemon, "Survie", json!(ALICE)).await;
    let path = format!("/servers/{}/owner", id_of(&server));

    let moved = platform
        .call(Method::PUT, &path, Some(json!({ "owner_ref": BOB })))
        .await;

    assert_eq!(moved.status, StatusCode::OK, "{}", moved.json);
    assert_eq!(moved.json["owner_ref"], BOB);
    let audit = platform.admin.last_audit("server.owner_changed").await;
    assert_eq!(audit.target_id.as_deref(), Some(id_of(&server)));
    assert_eq!(audit.details, json!({ "from": ALICE, "to": BOB }));

    let internal = platform
        .call(Method::PUT, &path, Some(json!({ "owner_ref": null })))
        .await;
    assert_eq!(internal.status, StatusCode::OK);
    assert_eq!(internal.json["owner_ref"], Value::Null);
}

#[tokio::test]
async fn transfer_to_the_same_owner_changes_nothing() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let server = deploy(&platform, &mut daemon, "Survie", json!(ALICE)).await;

    let response = platform
        .call(
            Method::PUT,
            &format!("/servers/{}/owner", id_of(&server)),
            Some(json!({ "owner_ref": ALICE })),
        )
        .await;

    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(response.json["code"], "empty_patch");
}

#[tokio::test]
async fn transfer_refuses_a_malformed_owner_and_an_unknown_server() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let server = deploy(&platform, &mut daemon, "Survie", json!(ALICE)).await;

    let malformed = platform
        .call(
            Method::PUT,
            &format!("/servers/{}/owner", id_of(&server)),
            Some(json!({ "owner_ref": "../etc" })),
        )
        .await;
    assert_eq!(malformed.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(malformed.json["fields"][0]["code"], "invalid_owner_ref");

    let unknown = platform
        .call(
            Method::PUT,
            &format!("/servers/{}/owner", Uuid::now_v7()),
            Some(json!({ "owner_ref": BOB })),
        )
        .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn transfer_needs_the_servers_update_permission() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let server = deploy(&platform, &mut daemon, "Survie", json!(ALICE)).await;
    let path = format!("/servers/{}/owner", id_of(&server));
    let reader = platform
        .admin
        .account("reader@example.com", &[Permission::ServersRead])
        .await;
    let editor = platform
        .admin
        .account(
            "editor@example.com",
            &[Permission::ServersRead, Permission::ServersUpdate],
        )
        .await;
    let reader = platform.admin.login(&reader).await.access_token;
    let editor = platform.admin.login(&editor).await.access_token;

    let refused = platform
        .admin
        .call(
            Method::PUT,
            &path,
            Some(&reader),
            Some(json!({ "owner_ref": BOB })),
        )
        .await;
    let accepted = platform
        .admin
        .call(
            Method::PUT,
            &path,
            Some(&editor),
            Some(json!({ "owner_ref": BOB })),
        )
        .await;

    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(accepted.status, StatusCode::OK, "{}", accepted.json);
}

#[tokio::test]
async fn the_list_filters_by_owner() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let alice = deploy(&platform, &mut daemon, "Alice", json!(ALICE)).await;
    deploy(&platform, &mut daemon, "Bob", json!(BOB)).await;
    deploy(&platform, &mut daemon, "Interne", Value::Null).await;

    let all = platform.call(Method::GET, "/servers", None).await;
    let only_alice = platform
        .call(
            Method::GET,
            "/servers?owner_ref=cloud%3A6f1c2a9e-3b7d-4c8e-9a51-2d4f8b0c7e13",
            None,
        )
        .await;

    assert_eq!(all.json.as_array().unwrap().len(), 3);
    let ids: Vec<&str> = only_alice
        .json
        .as_array()
        .unwrap()
        .iter()
        .map(id_of)
        .collect();
    assert_eq!(ids, [id_of(&alice)]);
}
