//! Ressources minimales et recommandées des prefabs (ADR-0031) : catalogue, plafonds, création,
//! modification des ressources et mise à jour de version.

use std::io::Write;
use std::path::Path;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use zip::write::SimpleFileOptions;

use crate::support::platform::{FakeDaemon, Platform};

/// `minecraft-java` en 1.5.0, qui demande 3 Go au moins et en recommande 4.
fn minecraft_with_needs() -> Vec<u8> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../prefabs/minecraft-java");
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default();
    // Fins de ligne normalisées : un checkout Windows du sous-module peut être en CRLF.
    let manifest = std::fs::read_to_string(dir.join("prefab.yaml"))
        .unwrap()
        .replace("\r\n", "\n")
        .replace("version: 1.4.0", "version: 1.5.0")
        .replacen(
            "requires:\n  kuro: \">=0.2\"\n",
            "requires:\n  kuro: \">=0.2\"\n  resources:\n    memory_mb: { min: 3072, recommended: 4096 }\n",
            1,
        );
    assert!(manifest.contains("min: 3072"), "the needs were not added");
    writer.start_file("prefab.yaml", options).unwrap();
    writer.write_all(manifest.as_bytes()).unwrap();
    for lang in ["en", "fr"] {
        let text = std::fs::read_to_string(dir.join(format!("locales/{lang}.json"))).unwrap();
        writer
            .start_file(format!("locales/{lang}.json"), options)
            .unwrap();
        writer.write_all(text.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

async fn import_needs(platform: &Platform) {
    let imported = platform
        .admin
        .call_bytes(
            Method::POST,
            "/prefabs/import",
            &platform.token,
            minecraft_with_needs(),
            true,
        )
        .await;
    assert_eq!(imported.status, StatusCode::CREATED, "{}", imported.json);
}

fn version<'a>(list: &'a Value, version: &str) -> &'a Value {
    list.as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "minecraft-java" && p["version"] == version)
        .unwrap_or_else(|| panic!("minecraft-java {version} missing"))
}

fn create_body(daemon: &FakeDaemon, memory_mb: i32) -> Value {
    json!({
        "name": "Survie",
        "node_id": daemon.node.id.as_uuid(),
        "prefab_id": "minecraft-java",
        "prefab_version": "1.5.0",
        "memory_mb": memory_mb,
    })
}

#[tokio::test]
async fn the_catalog_shows_the_needs_and_a_minimum_above_a_ceiling() {
    let platform = Platform::start().await;
    import_needs(&platform).await;

    let before = platform.call(Method::GET, "/prefabs", None).await;
    let settings = platform
        .call(
            Method::PATCH,
            "/settings",
            Some(json!({ "max_memory_mb": 2048 })),
        )
        .await;
    let after = platform.call(Method::GET, "/prefabs", None).await;

    let needs = version(&before.json, "1.5.0");
    assert_eq!(
        needs["resources"],
        json!({ "memory_mb": { "min": 3072, "recommended": 4096 } })
    );
    assert_eq!(needs["deployable"], true);
    assert_eq!(needs["over_ceiling"], json!([]));
    assert_eq!(version(&before.json, "1.4.0")["resources"], json!({}));
    assert_eq!(settings.status, StatusCode::OK, "{}", settings.json);
    assert_eq!(version(&after.json, "1.5.0")["deployable"], false);
    assert_eq!(
        version(&after.json, "1.5.0")["over_ceiling"],
        json!(["memory_mb"])
    );
    assert_eq!(version(&after.json, "1.4.0")["deployable"], true);
}

#[tokio::test]
async fn a_server_below_the_minimum_is_refused_before_reaching_the_node() {
    let platform = Platform::start().await;
    let daemon = platform.online_node("node-1", "25565-25570").await;
    import_needs(&platform).await;

    let response = platform
        .call(Method::POST, "/servers", Some(create_body(&daemon, 2048)))
        .await;

    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(response.json["code"], "resource_below_prefab_minimum");
    assert_eq!(response.json["fields"][0]["field"], "memory_mb");
    assert_eq!(response.json["fields"][0]["params"]["min"], 3072);
    let list = platform.call(Method::GET, "/servers", None).await;
    assert_eq!(list.json, json!([]));
}

#[tokio::test]
async fn lowering_a_server_below_its_minimum_is_refused() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    import_needs(&platform).await;
    let (created, _) = tokio::join!(
        platform.call(Method::POST, "/servers", Some(create_body(&daemon, 3072))),
        daemon.accept(),
    );
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json);
    let id = created.json["id"].as_str().unwrap().to_owned();
    daemon.report(&id, "offline").await;
    platform.wait_for_state(&id, "offline").await;

    let lowered = platform
        .call(
            Method::PATCH,
            &format!("/servers/{id}/resources"),
            Some(json!({ "memory_mb": 2048 })),
        )
        .await;

    assert_eq!(lowered.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(lowered.json["code"], "resource_below_prefab_minimum");
    assert_eq!(lowered.json["fields"][0]["field"], "memory_mb");
    let server = platform
        .call(Method::GET, &format!("/servers/{id}"), None)
        .await;
    assert_eq!(server.json["memory_mb"], 3072);
}

#[tokio::test]
async fn an_upgrade_to_a_version_needing_more_memory_is_blocked() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    // Déployé en 1.4.0 avec 2 Go, sans minimum.
    let (server, _) = platform.deploy_minecraft(&mut daemon, json!({})).await;
    let id = server["id"].as_str().unwrap().to_owned();
    daemon.report(&id, "offline").await;
    platform.wait_for_state(&id, "offline").await;
    import_needs(&platform).await;

    let preview = platform
        .call(
            Method::GET,
            &format!("/servers/{id}/upgrade?to=1.5.0"),
            None,
        )
        .await;

    assert_eq!(preview.status, StatusCode::OK, "{}", preview.json);
    assert_eq!(
        preview.json["blockers"],
        json!([{
            "kind": "resource_below_minimum",
            "resource": "memory_mb",
            "minimum": 3072,
            "current": 2048,
        }])
    );
}
