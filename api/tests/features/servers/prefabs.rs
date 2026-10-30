//! Catalogue des prefabs : import des officiels, lecture, traductions.

use axum::http::{Method, StatusCode};
use kuro_api::iam::domain::Permission;
use kuro_api::prefabs::application::import_dir;
use std::path::Path;

use crate::support::admin::TestAdmin;
use crate::support::platform::Platform;

#[tokio::test]
async fn official_prefabs_are_imported_once() {
    let platform = Platform::start().await;
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../prefabs");

    let again = import_dir(&platform.api.pool, &dir).await.unwrap();

    assert!(again.imported.is_empty());
    // minecraft-java, minecraft-bedrock, wordpress, ark-survival-ascended, terraria, valheim,
    // project-zomboid, rust, enshrouded, arma-3, fivem.
    assert_eq!(again.unchanged.len(), 11);
    let events = platform.admin.audit().await;
    let imports = events
        .iter()
        .filter(|e| e.action == "prefab.imported")
        .count();
    assert_eq!(imports, 11);
}

#[tokio::test]
async fn the_catalog_lists_names_and_deployability() {
    let platform = Platform::start().await;

    let list = platform.call(Method::GET, "/prefabs", None).await;

    assert_eq!(list.status, StatusCode::OK);
    let minecraft = list
        .json
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "minecraft-java")
        .unwrap();
    assert_eq!(minecraft["version"], "1.4.0");
    assert_eq!(minecraft["category"], "game");
    assert_eq!(minecraft["names"]["fr"], "Minecraft Java Edition");
    assert_eq!(minecraft["deployable"], true);
    let ark = list
        .json
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "ark-survival-ascended")
        .unwrap();
    // Un brouillon se déploie, mais sur un nœud de test seulement (ADR-0011).
    assert_eq!(ark["deployable"], true);
    assert_eq!(ark["test_nodes_only"], true);
    assert!(
        ark["draft_reason"]
            .as_str()
            .unwrap()
            .contains("not yet tried")
    );
    assert_eq!(minecraft["test_nodes_only"], false);
    let terraria = list
        .json
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "terraria")
        .unwrap();
    assert_eq!(terraria["deployable"], true, "install steps are supported");
}

#[tokio::test]
async fn the_manifest_keeps_the_display_order() {
    let platform = Platform::start().await;

    let prefab = platform
        .call(Method::GET, "/prefabs/minecraft-java/1.4.0", None)
        .await;

    assert_eq!(prefab.status, StatusCode::OK);
    // Ordre du fichier, vérifié sur le corps brut (`Value` trie les clés).
    let position = |needle: &str| {
        prefab
            .body
            .find(needle)
            .unwrap_or_else(|| panic!("{needle} missing"))
    };
    let order = [
        "\"server\":{\"icon\"",
        "\"version\":{\"icon\"",
        "\"world\":{\"icon\"",
        "\"security\":{\"icon\"",
    ];
    assert!(
        order.windows(2).all(|w| position(w[0]) < position(w[1])),
        "groups out of order"
    );
    let fields = [
        "\"motd\":{\"type\"",
        "\"max_players\":{\"type\"",
        "\"difficulty\":{\"type\"",
        "\"gamemode\":{\"type\"",
        "\"whitelist\":{\"type\"",
    ];
    assert!(
        fields.windows(2).all(|w| position(w[0]) < position(w[1])),
        "fields out of order"
    );
    assert_eq!(prefab.json["locales"], serde_json::json!(["en", "fr"]));
}

#[tokio::test]
async fn translations_are_served_per_language() {
    let platform = Platform::start().await;

    let fr = platform
        .call(
            Method::GET,
            "/prefabs/minecraft-java/1.4.0/locales/fr",
            None,
        )
        .await;
    let missing = platform
        .call(
            Method::GET,
            "/prefabs/minecraft-java/1.4.0/locales/xx",
            None,
        )
        .await;
    let unknown = platform
        .call(Method::GET, "/prefabs/ghost/1.0.0", None)
        .await;

    assert_eq!(fr.json["fields"]["motd"]["label"], "Message d'accueil");
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn reading_prefabs_requires_its_permission() {
    let admin = TestAdmin::start().await;
    let account = admin
        .account("alice@example.com", &[Permission::ServersRead])
        .await;
    let session = admin.login(&account).await;

    let response = admin
        .call(Method::GET, "/prefabs", Some(&session.access_token), None)
        .await;

    assert_eq!(response.status, StatusCode::FORBIDDEN);
    assert_eq!(response.json["params"]["permission"], "prefabs.read");
}

const ICON: &[u8] = b"\x89PNG\r\n\x1a\nkuro-icon";
const OTHER_ICON: &[u8] = b"\x89PNG\r\n\x1a\nkuro-other";
const COVER: &[u8] = b"RIFF\0\0\0\0WEBPkuro-cover";

/// Prefab `art-test` (copie de Terraria) qui déclare une icône et une bannière, dans un dossier
/// d'import à part.
fn art_prefab(icon: Option<&[u8]>, cover: Option<&[u8]>) -> tempfile::TempDir {
    let official = Path::new(env!("CARGO_MANIFEST_DIR")).join("../prefabs/terraria");
    let root = tempfile::tempdir().unwrap();
    write_art(root.path(), &official, icon, cover);
    root
}

/// Manifest sans sa rubrique `assets` (bloc de premier niveau et ses lignes indentées).
fn without_assets(manifest: &str) -> String {
    let mut skipping = false;
    manifest
        .lines()
        .filter(|line| {
            if *line == "assets:" {
                skipping = true;
                return false;
            }
            if skipping && line.starts_with("  ") {
                return false;
            }
            skipping = false;
            true
        })
        .map(|line| {
            format!(
                "{line}
"
            )
        })
        .collect()
}

fn write_art(root: &Path, official: &Path, icon: Option<&[u8]>, cover: Option<&[u8]>) {
    let dir = root.join("art");
    std::fs::create_dir_all(dir.join("locales")).unwrap();
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    for lang in ["en", "fr"] {
        let file = format!("locales/{lang}.json");
        std::fs::copy(official.join(&file), dir.join(&file)).unwrap();
    }
    // Les images déclarées par le prefab officiel sont retirées : le test déclare les siennes.
    let manifest = without_assets(&std::fs::read_to_string(official.join("prefab.yaml")).unwrap())
        .replace(
            "id: terraria",
            "id: art-test\nassets:\n  icon: assets/icon.png\n  cover: assets/cover.webp",
        );
    std::fs::write(dir.join("prefab.yaml"), manifest).unwrap();
    for (name, data) in [("icon.png", icon), ("cover.webp", cover)] {
        if let Some(data) = data {
            std::fs::write(dir.join("assets").join(name), data).unwrap();
        }
    }
}

#[tokio::test]
async fn prefab_images_are_imported_and_served() {
    let platform = Platform::start().await;
    let dir = art_prefab(Some(ICON), None);

    let report = import_dir(&platform.api.pool, dir.path()).await.unwrap();
    let list = platform.call(Method::GET, "/prefabs", None).await;
    let icon = platform
        .call(Method::GET, "/prefabs/art-test/1.0.0/assets/icon", None)
        .await;
    let cover = platform
        .call(Method::GET, "/prefabs/art-test/1.0.0/assets/cover", None)
        .await;
    let unknown = platform
        .call(Method::GET, "/prefabs/art-test/1.0.0/assets/banner", None)
        .await;

    assert_eq!(report.imported, vec!["art-test@1.0.0"]);
    let art = list
        .json
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "art-test")
        .unwrap();
    assert_eq!(art["assets"], serde_json::json!(["icon"]));
    assert_eq!(icon.status, StatusCode::OK);
    assert_eq!(icon.content_type.as_deref(), Some("image/png"));
    assert_eq!(icon.headers["x-content-type-options"], "nosniff");
    assert_eq!(
        icon.headers["content-security-policy"],
        "default-src 'none'; sandbox"
    );
    assert!(icon.body.ends_with("kuro-icon"));
    assert_eq!(
        cover.status,
        StatusCode::NOT_FOUND,
        "a declared image may be missing"
    );
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    let event = platform.admin.last_audit("prefab.imported").await;
    assert_eq!(event.details["prefab"], "art-test");
    assert_eq!(event.details["assets"], serde_json::json!(["icon"]));
}

#[tokio::test]
async fn a_missing_image_is_added_later_and_none_is_replaced() {
    let platform = Platform::start().await;
    let official = Path::new(env!("CARGO_MANIFEST_DIR")).join("../prefabs/terraria");
    let dir = art_prefab(Some(ICON), None);
    import_dir(&platform.api.pool, dir.path()).await.unwrap();

    write_art(dir.path(), &official, Some(OTHER_ICON), Some(COVER));
    let again = import_dir(&platform.api.pool, dir.path()).await.unwrap();
    let icon = platform
        .call(Method::GET, "/prefabs/art-test/1.0.0/assets/icon", None)
        .await;
    let cover = platform
        .call(Method::GET, "/prefabs/art-test/1.0.0/assets/cover", None)
        .await;

    assert_eq!(again.unchanged, vec!["art-test@1.0.0"]);
    assert!(icon.body.ends_with("kuro-icon"), "a stored image is kept");
    assert_eq!(cover.status, StatusCode::OK);
    assert_eq!(cover.content_type.as_deref(), Some("image/webp"));
    let event = platform.admin.last_audit("prefab.assets_imported").await;
    assert_eq!(event.details["prefab"], "art-test");
    assert_eq!(event.details["assets"], serde_json::json!(["cover"]));
}

#[tokio::test]
async fn an_image_that_is_not_png_jpeg_or_webp_is_refused() {
    let platform = Platform::start().await;
    let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\"><script>alert(1)</script></svg>";
    let dir = art_prefab(Some(ICON), Some(svg));

    let report = import_dir(&platform.api.pool, dir.path()).await.unwrap();
    let prefab = platform
        .call(Method::GET, "/prefabs/art-test/1.0.0", None)
        .await;

    assert!(report.imported.is_empty());
    assert_eq!(report.invalid.len(), 1);
    assert!(
        report.invalid[0].1.contains("cover.webp"),
        "{:?}",
        report.invalid
    );
    assert_eq!(prefab.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn reading_prefab_images_requires_its_permission() {
    let platform = Platform::start().await;
    let dir = art_prefab(Some(ICON), None);
    import_dir(&platform.api.pool, dir.path()).await.unwrap();
    let account = platform
        .admin
        .account("alice@example.com", &[Permission::ServersRead])
        .await;
    let session = platform.admin.login(&account).await;

    let response = platform
        .admin
        .call(
            Method::GET,
            "/prefabs/art-test/1.0.0/assets/icon",
            Some(&session.access_token),
            None,
        )
        .await;

    assert_eq!(response.status, StatusCode::FORBIDDEN);
    assert_eq!(response.json["params"]["permission"], "prefabs.read");
}

#[tokio::test]
async fn declaring_a_new_image_later_is_not_a_new_version() {
    // Les images sont cosmétiques : ajouter une vignette au manifest d'un prefab déjà importé ne
    // le fait pas passer pour modifié, et l'image est ajoutée.
    let platform = Platform::start().await;
    let dir = art_prefab(Some(ICON), None);
    import_dir(&platform.api.pool, dir.path()).await.unwrap();

    let manifest = dir.path().join("art/prefab.yaml");
    let text = std::fs::read_to_string(&manifest).unwrap().replace(
        "  cover: assets/cover.webp",
        "  cover: assets/cover.webp\n  thumbnail: assets/thumbnail.jpg",
    );
    std::fs::write(&manifest, text).unwrap();
    std::fs::write(
        dir.path().join("art/assets/thumbnail.jpg"),
        [0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10],
    )
    .unwrap();
    let again = import_dir(&platform.api.pool, dir.path()).await.unwrap();

    assert_eq!(again.unchanged, vec!["art-test@1.0.0"]);
    assert!(again.conflicts.is_empty());
    let thumbnail = platform
        .call(
            Method::GET,
            "/prefabs/art-test/1.0.0/assets/thumbnail",
            None,
        )
        .await;
    assert_eq!(thumbnail.status, StatusCode::OK);
    assert_eq!(thumbnail.content_type.as_deref(), Some("image/jpeg"));
    let list = platform.call(Method::GET, "/prefabs", None).await;
    let entry = list
        .json
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "art-test")
        .unwrap();
    assert!(
        entry["assets"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("thumbnail"))
    );
}

/// Serveur ARK (prefab officiel en brouillon) sur le nœud donné.
fn ark(daemon: &crate::support::platform::FakeDaemon) -> serde_json::Value {
    serde_json::json!({
        "name": "Ark",
        "node_id": daemon.node.id.as_uuid(),
        "prefab_id": "ark-survival-ascended",
        "memory_mb": 16384,
        "cpu_percent": 400,
        "disk_mb": 51200,
    })
}

#[tokio::test]
async fn a_draft_version_deploys_on_a_test_node_only() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "7777-7780").await;

    let refused = platform
        .call(Method::POST, "/servers", Some(ark(&daemon)))
        .await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        refused.json["fields"][0]["field"], "node_id",
        "{}",
        refused.json
    );
    assert_eq!(
        refused.json["fields"][0]["code"],
        "prefab_draft_needs_test_node"
    );

    let marked = platform
        .call(
            Method::PATCH,
            &format!("/nodes/{}", daemon.node.id.as_uuid()),
            Some(serde_json::json!({ "trials": true })),
        )
        .await;
    assert_eq!(marked.status, StatusCode::OK, "{}", marked.json);
    let (created, deploy) = tokio::join!(
        platform.call(Method::POST, "/servers", Some(ark(&daemon))),
        daemon.accept(),
    );
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json);
    assert_eq!(deploy["type"], "server.deploy");
    // Serveur ordinaire : pas d'expiration, contrairement à un essai du Studio.
    assert_eq!(
        created.json["trial"],
        serde_json::Value::Null,
        "{}",
        created.json
    );
}

#[tokio::test]
async fn a_tcp_and_udp_port_gets_one_allocation_published_twice() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "30120-30125").await;
    platform
        .call(
            Method::PATCH,
            &format!("/nodes/{}", daemon.node.id.as_uuid()),
            Some(serde_json::json!({ "trials": true })),
        )
        .await;

    let (created, deploy) = tokio::join!(
        platform.call(
            Method::POST,
            "/servers",
            Some(serde_json::json!({
                "name": "FiveM",
                "node_id": daemon.node.id.as_uuid(),
                "prefab_id": "fivem",
                "memory_mb": 4096,
                "cpu_percent": 200,
                "disk_mb": 10240,
                "settings": { "license": "cfxk_test_licence" },
            })),
        ),
        daemon.accept(),
    );
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json);
    // Une seule allocation pour le jeu, une pour txAdmin.
    assert_eq!(created.json["allocations"].as_array().unwrap().len(), 2);
    let ports = deploy["payload"]["services"][0]["ports"]
        .as_array()
        .unwrap();
    let game: Vec<_> = ports.iter().filter(|p| p["name"] == "game").collect();
    assert_eq!(game.len(), 2, "{ports:?}");
    assert_eq!(game[0]["protocol"], "tcp");
    assert_eq!(game[1]["protocol"], "udp");
    assert_eq!(game[0]["published"], game[1]["published"]);
    assert_eq!(game[0]["container_port"], 30120);
}
