//! `GET /servers` : par nom, fenêtre `limit`/`offset`, ports de chaque serveur en une requête.

use axum::http::{Method, StatusCode};
use serde_json::json;

use crate::support::platform::Platform;

async fn names(platform: &Platform, query: &str) -> Vec<String> {
    let response = platform
        .call(Method::GET, &format!("/servers{query}"), None)
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    response.json["items"]
        .as_array()
        .or_else(|| response.json.as_array())
        .unwrap()
        .iter()
        .map(|server| server["name"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn the_list_is_by_name_and_can_be_windowed() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    for name in ["Survie", "Créatif", "Arène"] {
        let (response, _) = tokio::join!(
            platform.call(
                Method::POST,
                "/servers",
                Some(json!({
                    "name": name,
                    "node_id": daemon.node.id.as_uuid(),
                    "prefab_id": "minecraft-java",
                    "memory_mb": 2048,
                    "cpu_percent": 200,
                    "disk_mb": 10240,
                    "settings": {},
                })),
            ),
            daemon.accept(),
        );
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.json);
    }

    assert_eq!(names(&platform, "").await, ["Arène", "Créatif", "Survie"]);
    assert_eq!(names(&platform, "?limit=2").await, ["Arène", "Créatif"]);
    assert_eq!(names(&platform, "?limit=2&offset=2").await, ["Survie"]);
    assert_eq!(names(&platform, "?offset=3").await, Vec::<String>::new());

    // Chaque serveur de la liste a ses ports.
    let response = platform.call(Method::GET, "/servers", None).await;
    for server in response.json.as_array().unwrap() {
        assert_eq!(
            server["allocations"].as_array().map(Vec::len),
            Some(1),
            "{server}"
        );
    }
}
