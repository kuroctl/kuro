//! Store de prefabs (ADR-0042) : index signé lu et vérifié, versions annoncées avec leur
//! compatibilité et leur présence au catalogue, installation d'une archive vérifiée par sa
//! somme, refus d'un index signé par une autre clé ou d'une archive altérée.

use axum::http::{Method, StatusCode};
use kuro_prefab::store::{IndexEntry, StoreIndex};
use serde_json::json;

use crate::support::platform::Platform;

fn index(entries: Vec<IndexEntry>) -> StoreIndex {
    StoreIndex::new("2026-10-30T12:00:00Z".to_owned(), entries)
}

#[tokio::test]
async fn the_store_lists_its_versions_with_compatibility_and_catalog_presence() {
    let platform = Platform::start().await;
    let store = &platform.admin.store;
    // Une version nouvelle, une déjà au catalogue (importée du dossier officiel), une qui
    // exige un Kuro futur.
    let new = store.publish_folder("wordpress", |m| {
        m.replace("version: 1.2.0", "version: 9.0.0")
    });
    let present = store.publish_folder("wordpress", |m| m.to_owned());
    let mut future = store.publish_folder("terraria", |m| {
        m.replace("version: 1.0.0", "version: 9.0.0")
    });
    future.requires_kuro = Some(">=99".to_owned());
    store.publish(&index(vec![future, new, present]));

    let response = platform.call(Method::GET, "/prefabs/stores", None).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    let stores = response.json.as_array().unwrap();
    assert_eq!(stores.len(), 1);
    assert_eq!(stores[0]["name"], "test");
    assert!(stores[0]["error"].is_null(), "{}", stores[0]);
    let prefabs = stores[0]["prefabs"].as_array().unwrap();
    let by = |id: &str, version: &str| {
        prefabs
            .iter()
            .find(|p| p["id"] == id && p["version"] == version)
            .unwrap_or_else(|| panic!("{id}@{version} missing in {prefabs:?}"))
    };
    let wp_new = by("wordpress", "9.0.0");
    assert_eq!(wp_new["compatible"], true);
    assert_eq!(wp_new["installed"], false);
    assert_eq!(wp_new["category"], "web");
    assert!(
        wp_new["names"]["en"]
            .as_str()
            .unwrap()
            .contains("WordPress")
    );
    assert!(
        wp_new["assets"]
            .as_array()
            .unwrap()
            .contains(&json!("icon"))
    );
    assert_eq!(by("wordpress", "1.2.0")["installed"], true);
    let terraria = by("terraria", "9.0.0");
    assert_eq!(terraria["compatible"], false);
    assert_eq!(terraria["requires_kuro"], ">=99");
    // L'index et sa signature ont été lus, sans jeton.
    let requests = store.requests.lock().unwrap().clone();
    assert!(
        requests
            .iter()
            .any(|(url, token)| url.ends_with("/index.json") && token.is_none())
    );
    assert!(
        requests
            .iter()
            .any(|(url, _)| url.ends_with("/index.json.minisig"))
    );
}

#[tokio::test]
async fn installing_from_the_store_checks_the_archive_and_imports_it_as_store() {
    let platform = Platform::start().await;
    let store = &platform.admin.store;
    let entry = store.publish_folder("wordpress", |m| {
        m.replace("version: 1.2.0", "version: 9.0.0")
    });
    store.publish(&index(vec![entry.clone()]));

    let installed = platform
        .call(
            Method::POST,
            "/prefabs/stores/test/install",
            Some(json!({ "id": "wordpress", "version": "9.0.0" })),
        )
        .await;
    assert_eq!(installed.status, StatusCode::CREATED, "{}", installed.json);
    assert_eq!(installed.json["source"], "store");
    assert_eq!(installed.json["store"], "test");
    assert!(
        installed.json["assets"]
            .as_array()
            .unwrap()
            .contains(&json!("icon"))
    );
    let event = platform.admin.last_audit("prefab.imported").await;
    assert_eq!(event.details["source"], "store");
    assert_eq!(event.details["store"], "test");
    assert_eq!(event.details["prefab"], "wordpress");
    // Au catalogue, marquée installée dans le store.
    let stores = platform.call(Method::GET, "/prefabs/stores", None).await;
    assert_eq!(stores.json[0]["prefabs"][0]["installed"], true);

    // Une seconde fois : la version est immuable.
    let again = platform
        .call(
            Method::POST,
            "/prefabs/stores/test/install",
            Some(json!({ "id": "wordpress", "version": "9.0.0" })),
        )
        .await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    assert_eq!(again.json["code"], "prefab_exists");

    // Hors de l'index, ou store inconnu : introuvable.
    let missing = platform
        .call(
            Method::POST,
            "/prefabs/stores/test/install",
            Some(json!({ "id": "wordpress", "version": "9.9.9" })),
        )
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    let unknown = platform
        .call(
            Method::POST,
            "/prefabs/stores/nope/install",
            Some(json!({ "id": "wordpress", "version": "9.0.0" })),
        )
        .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_altered_archive_is_refused() {
    let platform = Platform::start().await;
    let store = &platform.admin.store;
    let entry = store.publish_folder("terraria", |m| {
        m.replace("version: 1.0.0", "version: 9.0.0")
    });
    store.publish(&index(vec![entry.clone()]));
    // L'archive change après la publication de l'index : sa somme ne correspond plus.
    let tampered = store.publish_folder("terraria", |m| {
        m.replace("version: 1.0.0", "version: 9.0.0")
            .replace("tags: [", "tags: [altered, ")
    });
    assert_ne!(tampered.sha256, entry.sha256);

    let refused = platform
        .call(
            Method::POST,
            "/prefabs/stores/test/install",
            Some(json!({ "id": "terraria", "version": "9.0.0" })),
        )
        .await;
    assert_eq!(refused.status, StatusCode::BAD_GATEWAY, "{}", refused.json);
    assert_eq!(refused.json["code"], "store_corrupted");
    let catalog = platform.call(Method::GET, "/prefabs", None).await;
    assert!(
        !catalog
            .json
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == "terraria" && p["version"] == "9.0.0")
    );
}

#[tokio::test]
async fn an_index_signed_by_another_key_or_unreachable_is_not_trusted() {
    let platform = Platform::start().await;
    let store = &platform.admin.store;
    let entry = store.publish_folder("terraria", |m| {
        m.replace("version: 1.0.0", "version: 9.0.0")
    });
    store.publish_with_other_key(&index(vec![entry.clone()]));

    let stores = platform.call(Method::GET, "/prefabs/stores", None).await;
    assert_eq!(stores.status, StatusCode::OK);
    let error = stores.json[0]["error"].as_str().unwrap();
    assert!(error.contains("signature"), "{error}");
    assert_eq!(stores.json[0]["prefabs"], json!([]));
    let refused = platform
        .call(
            Method::POST,
            "/prefabs/stores/test/install",
            Some(json!({ "id": "terraria", "version": "9.0.0" })),
        )
        .await;
    assert_eq!(refused.status, StatusCode::BAD_GATEWAY, "{}", refused.json);
    assert_eq!(refused.json["code"], "store_untrusted");

    // Store en panne : erreur rendue, les autres lectures ne sont pas bloquées.
    *store.unavailable.lock().unwrap() = true;
    let down = platform.call(Method::GET, "/prefabs/stores", None).await;
    assert!(
        down.json[0]["error"]
            .as_str()
            .unwrap()
            .contains("store down")
    );
    *store.unavailable.lock().unwrap() = false;

    // Signé par la bonne clé : lu, puis gardé en cache jusqu'à `refresh`.
    store.publish(&index(vec![entry]));
    let ok = platform.call(Method::GET, "/prefabs/stores", None).await;
    assert!(ok.json[0]["error"].is_null(), "{}", ok.json);
    assert_eq!(ok.json[0]["prefabs"][0]["id"], "terraria");
    store.publish(&index(Vec::new()));
    let cached = platform.call(Method::GET, "/prefabs/stores", None).await;
    assert_eq!(cached.json[0]["prefabs"].as_array().unwrap().len(), 1);
    let refreshed = platform
        .call(Method::GET, "/prefabs/stores?refresh=true", None)
        .await;
    assert_eq!(refreshed.json[0]["prefabs"], json!([]));
}
