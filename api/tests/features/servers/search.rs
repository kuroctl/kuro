//! Liste paginée des serveurs et actions groupées (ADR-0025) : recherche, filtres, facettes,
//! tri, pagination, « tout sélectionner », actions par serveur avec leur résultat.

use axum::http::{Method, StatusCode};
use kuro_api::iam::domain::Permission;
use serde_json::{Value, json};

use crate::support::platform::{FakeDaemon, Platform};

async fn deploy(platform: &Platform, daemon: &mut FakeDaemon, name: &str, prefab: &str) -> String {
    let body = json!({
        "name": name,
        "node_id": daemon.node.id.as_uuid(),
        "prefab_id": prefab,
        "memory_mb": 2048,
        "cpu_percent": 200,
        "disk_mb": 10240,
    });
    let (created, _) = tokio::join!(
        platform.call(Method::POST, "/servers", Some(body)),
        daemon.accept(),
    );
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json);
    created.json["id"].as_str().unwrap().to_owned()
}

async fn search(platform: &Platform, query: &str) -> Value {
    let response = platform
        .call(Method::GET, &format!("/servers/search?{query}"), None)
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    response.json
}

fn names(page: &Value) -> Vec<&str> {
    page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn the_list_is_searched_filtered_sorted_and_paged_by_the_api() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25580").await;
    let alpha = deploy(&platform, &mut daemon, "Alpha", "minecraft-java").await;
    deploy(&platform, &mut daemon, "Bravo", "minecraft-java").await;
    deploy(&platform, &mut daemon, "Charlie", "terraria").await;
    daemon.report(&alpha, "running").await;
    platform.wait_for_state(&alpha, "running").await;

    let all = search(&platform, "").await;
    assert_eq!(all["total"], 3);
    assert_eq!(names(&all), ["Alpha", "Bravo", "Charlie"]);
    let prefabs: Vec<(&str, u64)> = all["facets"]["prefab"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["value"].as_str().unwrap(), f["count"].as_u64().unwrap()))
        .collect();
    assert_eq!(prefabs, [("minecraft-java", 2), ("terraria", 1)]);

    let minecraft = search(&platform, "prefab=minecraft-java&sort=name&order=desc").await;
    assert_eq!(names(&minecraft), ["Bravo", "Alpha"]);
    // La facette du prefab compte encore les autres jeux, celle de l'état non.
    assert_eq!(minecraft["facets"]["prefab"].as_array().unwrap().len(), 2);

    let running = search(&platform, "state=running").await;
    assert_eq!(names(&running), ["Alpha"]);
    let found = search(&platform, "q=char").await;
    assert_eq!(names(&found), ["Charlie"]);

    let page = search(&platform, "per_page=2&page=2&all_ids=true").await;
    assert_eq!(names(&page), ["Charlie"]);
    assert_eq!(page["total"], 3);
    assert_eq!(page["ids"].as_array().unwrap().len(), 3);

    let wrong = platform
        .call(Method::GET, "/servers/search?sort=nope", None)
        .await;
    assert_eq!(wrong.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn a_bulk_action_reports_each_server_and_audits_each_one() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25580").await;
    let alpha = deploy(&platform, &mut daemon, "Alpha", "minecraft-java").await;
    let bravo = deploy(&platform, &mut daemon, "Bravo", "minecraft-java").await;
    for id in [&alpha, &bravo] {
        daemon.report(id, "offline").await;
        platform.wait_for_state(id, "offline").await;
    }
    let suspend = format!("/servers/{bravo}/suspend");
    let (suspended, _) =
        tokio::join!(platform.call(Method::POST, &suspend, None), daemon.accept(),);
    assert_eq!(suspended.status, StatusCode::OK, "{}", suspended.json);

    let (response, command) = tokio::join!(
        platform.call(
            Method::POST,
            "/servers/bulk",
            Some(json!({ "action": "start", "servers": [alpha, bravo] })),
        ),
        daemon.accept(),
    );
    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(command["type"], "server.power");
    assert_eq!(
        response.json,
        json!([
            { "server_id": alpha, "ok": true, "error": null },
            { "server_id": bravo, "ok": false, "error": "server_suspended" },
        ])
    );
    let event = platform.admin.last_audit("server.power").await;
    assert_eq!(event.target_id.as_deref(), Some(alpha.as_str()));
}

async fn bulk(
    platform: &Platform,
    token: &str,
    action: &str,
    id: uuid::Uuid,
) -> crate::support::http::TestResponse {
    platform
        .admin
        .call(
            Method::POST,
            "/servers/bulk",
            Some(token),
            Some(json!({ "action": action, "servers": [id] })),
        )
        .await
}

#[tokio::test]
async fn a_bulk_action_needs_the_permission_of_the_action() {
    let platform = Platform::start().await;
    let account = platform
        .admin
        .account(
            "reader@example.com",
            &[Permission::ServersRead, Permission::ServersPower],
        )
        .await;
    let token = platform.admin.login(&account).await.access_token;
    let id = uuid::Uuid::now_v7();
    let suspend = bulk(&platform, &token, "suspend", id).await;
    assert_eq!(suspend.status, StatusCode::FORBIDDEN);
    assert_eq!(suspend.json["params"]["permission"], "servers.suspend");
    let start = bulk(&platform, &token, "start", id).await;
    assert_eq!(start.status, StatusCode::OK);
    assert_eq!(start.json[0]["error"], "not_found");

    let empty = platform
        .call(
            Method::POST,
            "/servers/bulk",
            Some(json!({ "action": "start", "servers": [] })),
        )
        .await;
    assert_eq!(empty.status, StatusCode::UNPROCESSABLE_ENTITY);
}
