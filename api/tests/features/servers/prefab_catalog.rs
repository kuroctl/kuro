//! Gestion du catalogue des prefabs : import d'une archive `.kprefab` (validation, immuabilité,
//! refus journalisé), origine et nombre de serveurs, suppression d'une version inutilisée,
//! permissions.

use std::io::Write;
use std::path::Path;

use axum::http::{Method, StatusCode};
use kuro_api::iam::domain::Permission;
use serde_json::json;
use zip::write::SimpleFileOptions;

use crate::support::platform::Platform;

/// Archive du prefab officiel `terraria`, version remplacée par `version`.
fn terraria(version: &str) -> Vec<u8> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../prefabs/terraria");
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default();
    let manifest = std::fs::read_to_string(dir.join("prefab.yaml"))
        .unwrap()
        .replace("version: 1.0.0", &format!("version: {version}"));
    writer.start_file("prefab.yaml", options).unwrap();
    writer.write_all(manifest.as_bytes()).unwrap();
    for entry in std::fs::read_dir(dir.join("locales")).unwrap() {
        let path = entry.unwrap().path();
        let name = format!("locales/{}", path.file_name().unwrap().to_string_lossy());
        writer.start_file(name, options).unwrap();
        writer.write_all(&std::fs::read(&path).unwrap()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

async fn import(platform: &Platform, bytes: Vec<u8>) -> crate::support::http::TestResponse {
    platform
        .admin
        .call_bytes(
            Method::POST,
            "/prefabs/import",
            &platform.token,
            bytes,
            true,
        )
        .await
}

#[tokio::test]
async fn an_archive_is_imported_as_a_new_version_and_audited() {
    let platform = Platform::start().await;

    let imported = import(&platform, terraria("2.0.0")).await;

    assert_eq!(imported.status, StatusCode::CREATED, "{}", imported.json);
    assert_eq!(imported.json["id"], "terraria");
    assert_eq!(imported.json["version"], "2.0.0");
    assert_eq!(imported.json["source"], "uploaded");
    assert_eq!(imported.json["servers"], 0);
    let list = platform.call(Method::GET, "/prefabs", None).await;
    let versions: Vec<_> = list
        .json
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["id"] == "terraria")
        .map(|p| (p["version"].clone(), p["source"].clone()))
        .collect();
    assert_eq!(
        versions,
        vec![
            (json!("1.0.0"), json!("official")),
            (json!("2.0.0"), json!("uploaded"))
        ]
    );
    let event = platform.admin.last_audit("prefab.imported").await;
    assert_eq!(event.outcome, "success");
    assert_eq!(event.actor_kind, "admin");
    assert_eq!(event.details["source"], "uploaded");

    // `id` + `version` est immuable.
    let again = import(&platform, terraria("2.0.0")).await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    assert_eq!(again.json["code"], "prefab_exists");
}

#[tokio::test]
async fn an_invalid_archive_is_refused_with_its_issues_and_audited() {
    let platform = Platform::start().await;

    let garbage = import(&platform, b"not a zip at all".to_vec()).await;
    assert_eq!(garbage.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(garbage.json["code"], "invalid_prefab");
    assert!(
        garbage.json["params"]["reason"]
            .as_str()
            .unwrap()
            .contains("zip")
    );

    let invalid = import(&platform, terraria("not-a-version")).await;
    assert_eq!(
        invalid.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        invalid.json
    );
    assert_eq!(invalid.json["code"], "invalid_prefab");

    let event = platform.admin.last_audit("prefab.imported").await;
    assert_eq!(event.outcome, "failure");
}

#[tokio::test]
async fn an_oversized_archive_never_reaches_the_import() {
    let platform = Platform::start().await;
    let huge = vec![0u8; kuro_prefab::files::MAX_ARCHIVE_BYTES + 2048];

    let response = import(&platform, huge).await;

    assert_eq!(response.status, StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn a_version_in_use_cannot_be_deleted_but_an_unused_one_can() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565").await;
    platform.deploy_minecraft(&mut daemon, json!({})).await;
    let minecraft = platform
        .call(Method::GET, "/prefabs", None)
        .await
        .json
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "minecraft-java")
        .cloned()
        .unwrap();
    assert_eq!(minecraft["servers"], 1);
    let path = format!(
        "/prefabs/minecraft-java/{}",
        minecraft["version"].as_str().unwrap()
    );

    let refused = platform.call(Method::DELETE, &path, None).await;
    assert_eq!(refused.status, StatusCode::CONFLICT);
    assert_eq!(refused.json["code"], "prefab_in_use");
    let servers = platform
        .call(Method::GET, &format!("{path}/servers"), None)
        .await;
    assert_eq!(servers.json[0]["name"], "Survie");

    let deleted = platform
        .call(Method::DELETE, "/prefabs/terraria/1.0.0", None)
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    let gone = platform
        .call(Method::GET, "/prefabs/terraria/1.0.0", None)
        .await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
    let event = platform.admin.last_audit("prefab.deleted").await;
    assert_eq!(event.details["prefab"], "terraria");
}

#[tokio::test]
async fn reading_the_catalog_is_not_managing_it() {
    let platform = Platform::start().await;
    let admin = &platform.admin;
    let account = admin
        .account("reader@example.com", &[Permission::PrefabsRead])
        .await;
    let token = admin.login(&account).await.access_token;

    let list = admin
        .call(Method::GET, "/prefabs", Some(&token), None)
        .await;
    assert_eq!(list.status, StatusCode::OK);
    let imported = admin
        .call_bytes(
            Method::POST,
            "/prefabs/import",
            &token,
            terraria("3.0.0"),
            true,
        )
        .await;
    assert_eq!(imported.status, StatusCode::FORBIDDEN);
    assert_eq!(imported.json["params"]["permission"], "prefabs.manage");
    let deleted = admin
        .call(
            Method::DELETE,
            "/prefabs/terraria/1.0.0",
            Some(&token),
            None,
        )
        .await;
    assert_eq!(deleted.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn reloading_adds_missing_images_without_a_restart_and_is_audited() {
    let platform = Platform::start().await;
    // Comme une image ajoutée au dossier après le démarrage de l'API.
    sqlx::query("DELETE FROM prefab_assets WHERE prefab_id = 'minecraft-java' AND kind = 'cover'")
        .execute(&platform.api.pool)
        .await
        .unwrap();

    let reloaded = platform.call(Method::POST, "/prefabs/reload", None).await;

    assert_eq!(reloaded.status, StatusCode::OK, "{}", reloaded.json);
    let added = reloaded.json["assets_added"].as_array().unwrap();
    assert!(
        added.iter().any(
            |entry| entry.as_str().unwrap().starts_with("minecraft-java@")
                && entry.as_str().unwrap().contains("cover")
        ),
        "{added:?}"
    );
    let event = platform.admin.last_audit("prefabs.reloaded").await;
    assert_eq!(event.actor_kind, "admin");
    let images = platform.admin.last_audit("prefab.assets_imported").await;
    assert_eq!(images.actor_kind, "admin");
}

#[tokio::test]
async fn reloading_requires_the_manage_permission() {
    let platform = Platform::start().await;
    let admin = &platform.admin;
    let account = admin
        .account("reader@example.com", &[Permission::PrefabsRead])
        .await;
    let token = admin.login(&account).await.access_token;

    let refused = admin
        .call(Method::POST, "/prefabs/reload", Some(&token), None)
        .await;

    assert_eq!(refused.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_watched_folder_is_imported_as_soon_as_it_changes() {
    let platform = Platform::start().await;
    let root = tempfile::tempdir().unwrap();
    let shutdown = tokio_util::sync::CancellationToken::new();
    let watcher = tokio::spawn(kuro_api::prefabs::application::watch_dir(
        platform.api.pool.clone(),
        root.path().to_path_buf(),
        shutdown.clone(),
    ));

    // Nouveau prefab déposé pendant que l'API tourne.
    let official = Path::new(env!("CARGO_MANIFEST_DIR")).join("../prefabs/terraria");
    let dir = root.path().join("watched");
    std::fs::create_dir_all(dir.join("locales")).unwrap();
    for lang in ["en", "fr"] {
        let file = format!("locales/{lang}.json");
        std::fs::copy(official.join(&file), dir.join(&file)).unwrap();
    }
    let manifest = std::fs::read_to_string(official.join("prefab.yaml"))
        .unwrap()
        .replace("id: terraria", "id: watched");
    std::fs::write(dir.join("prefab.yaml"), manifest).unwrap();

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        let found = platform
            .call(Method::GET, "/prefabs/watched/1.0.0", None)
            .await;
        if found.status == StatusCode::OK {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            let report =
                kuro_api::prefabs::application::import_dir(&platform.api.pool, root.path())
                    .await
                    .unwrap();
            panic!("watched prefab never imported: {report:?}");
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    shutdown.cancel();
    watcher.await.unwrap();
}

#[tokio::test]
async fn reloading_ignores_fields_added_to_the_model_and_adds_images_even_on_conflict() {
    let platform = Platform::start().await;
    let pool = &platform.api.pool;
    // Manifest stocké par une version antérieure du modèle (sans `backup`) : pas un conflit.
    sqlx::query(
        "UPDATE prefabs SET manifest = (manifest::jsonb - 'backup')::text WHERE id = 'wordpress'",
    )
    .execute(pool)
    .await
    .unwrap();
    // Vrai changement de contenu sans changement de version : la version stockée est gardée,
    // mais ses images manquantes sont ajoutées.
    sqlx::query(
        "UPDATE prefabs SET manifest = jsonb_set(manifest::jsonb, '{tags}', '[\"old\"]')::text
         WHERE id = 'terraria'",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("DELETE FROM prefab_assets WHERE prefab_id = 'terraria'")
        .execute(pool)
        .await
        .unwrap();

    let reloaded = platform.call(Method::POST, "/prefabs/reload", None).await;

    assert_eq!(reloaded.status, StatusCode::OK, "{}", reloaded.json);
    assert_eq!(reloaded.json["conflicts"], json!(["terraria@1.0.0"]));
    let unchanged = reloaded.json["unchanged"].as_array().unwrap();
    assert!(
        unchanged.contains(&json!("wordpress@1.2.0")),
        "{unchanged:?}"
    );
    let added = reloaded.json["assets_added"].as_array().unwrap();
    assert!(
        added
            .iter()
            .any(|entry| entry.as_str().unwrap().starts_with("terraria@1.0.0")),
        "{added:?}"
    );
}

/// PNG uni hors convention.
fn oversized_png(width: u32, height: u32) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        width,
        height,
        image::Rgba([200, 30, 30, 255]),
    ))
    .write_to(&mut out, image::ImageFormat::Png)
    .unwrap();
    out.into_inner()
}

#[tokio::test]
async fn images_are_brought_to_the_convention_and_revalidated_by_the_browser() {
    use tower::ServiceExt;

    let platform = Platform::start().await;
    sqlx::query(
        "UPDATE prefab_assets SET data = $1
         WHERE prefab_id = 'terraria' AND prefab_version = '1.0.0' AND kind = 'icon'",
    )
    .bind(oversized_png(1000, 800))
    .execute(&platform.api.pool)
    .await
    .unwrap();

    let optimized = platform
        .call(
            Method::POST,
            "/prefabs/terraria/1.0.0/assets/optimize",
            None,
        )
        .await;

    assert_eq!(optimized.status, StatusCode::OK, "{}", optimized.json);
    let icon = optimized.json["optimized"]
        .as_array()
        .unwrap()
        .iter()
        .find(|asset| asset["kind"] == "icon")
        .cloned()
        .unwrap();
    assert_eq!(icon["before"]["width"], 1000);
    assert_eq!(
        (
            icon["after"]["width"].clone(),
            icon["after"]["height"].clone()
        ),
        (json!(64), json!(64))
    );
    let event = platform.admin.last_audit("prefab.assets_optimized").await;
    assert_eq!(event.actor_kind, "admin");
    assert_eq!(event.details["prefab"], "terraria");

    // L'image servie est la nouvelle, revalidée par son empreinte.
    let served = platform
        .call(Method::GET, "/prefabs/terraria/1.0.0/assets/icon", None)
        .await;
    assert_eq!(served.status, StatusCode::OK);
    assert_eq!(served.headers["cache-control"], "private, no-cache");
    let etag = served.headers["etag"].clone();
    let request = axum::http::Request::get("/api/admin/v1/prefabs/terraria/1.0.0/assets/icon")
        .header("authorization", format!("Bearer {}", platform.token))
        .header("if-none-match", etag)
        .body(axum::body::Body::empty())
        .unwrap();
    let revalidated = platform.admin.router().oneshot(request).await.unwrap();
    assert_eq!(revalidated.status(), StatusCode::NOT_MODIFIED);

    // Une seconde fois : rien à refaire.
    let again = platform
        .call(
            Method::POST,
            "/prefabs/terraria/1.0.0/assets/optimize",
            None,
        )
        .await;
    assert!(
        again.json["unchanged"]
            .as_array()
            .unwrap()
            .contains(&json!("icon"))
    );
    assert_eq!(again.json["optimized"], json!([]));
}

#[tokio::test]
async fn optimizing_images_requires_the_manage_permission() {
    let platform = Platform::start().await;
    let admin = &platform.admin;
    let account = admin
        .account("reader@example.com", &[Permission::PrefabsRead])
        .await;
    let token = admin.login(&account).await.access_token;

    let refused = admin
        .call(
            Method::POST,
            "/prefabs/terraria/1.0.0/assets/optimize",
            Some(&token),
            None,
        )
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(refused.json["params"]["permission"], "prefabs.manage");

    let unknown = platform
        .call(Method::POST, "/prefabs/nope/1.0.0/assets/optimize", None)
        .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn declaring_the_steam_game_of_a_published_prefab_is_not_a_conflict() {
    let platform = Platform::start().await;
    // Version stockée avant que le prefab déclare son jeu Steam (ADR-0039).
    sqlx::query(
        "UPDATE prefabs SET manifest = (manifest::jsonb - 'steam')::text WHERE id = 'terraria'",
    )
    .execute(&platform.api.pool)
    .await
    .unwrap();

    let reloaded = platform.call(Method::POST, "/prefabs/reload", None).await;

    assert_eq!(reloaded.status, StatusCode::OK, "{}", reloaded.json);
    assert_eq!(reloaded.json["conflicts"], json!([]));
    let unchanged = reloaded.json["unchanged"].as_array().unwrap();
    assert!(
        unchanged.contains(&json!("terraria@1.0.0")),
        "{unchanged:?}"
    );
}

#[tokio::test]
async fn the_search_groups_versions_by_prefab_and_pages_the_results() {
    let platform = Platform::start().await;
    let imported = import(&platform, terraria("2.0.0")).await;
    assert_eq!(imported.status, StatusCode::CREATED, "{}", imported.json);

    // Par le nom traduit, sans casse : une entrée par prefab, versions décroissantes.
    let found = platform
        .call(Method::GET, "/prefabs/search?q=TERRA", None)
        .await;
    assert_eq!(found.status, StatusCode::OK, "{}", found.json);
    assert_eq!(found.json["total"], 1);
    assert_eq!(found.json["items"][0]["id"], "terraria");
    assert_eq!(found.json["items"][0]["names"]["fr"], "Terraria");
    assert_eq!(
        found.json["items"][0]["versions"],
        json!(["2.0.0", "1.0.0"])
    );

    // Sans recherche : tout le catalogue, trié par id, page par page.
    let all = platform.call(Method::GET, "/prefabs/search", None).await;
    let total = all.json["total"].as_u64().unwrap();
    assert!(total > 2);
    let second = platform
        .call(Method::GET, "/prefabs/search?page=2&per_page=2", None)
        .await;
    assert_eq!(second.json["total"], total);
    assert_eq!(second.json["per_page"], 2);
    assert_eq!(second.json["items"].as_array().unwrap().len(), 2);
    assert_eq!(second.json["items"][0]["id"], all.json["items"][2]["id"]);

    let none = platform
        .call(Method::GET, "/prefabs/search?q=zzz-nothing", None)
        .await;
    assert_eq!(none.json["total"], 0);
}

#[tokio::test]
async fn searching_the_catalog_requires_the_read_permission() {
    let platform = Platform::start().await;
    let admin = &platform.admin;
    let account = admin
        .account("operator@example.com", &[Permission::ServersRead])
        .await;
    let token = admin.login(&account).await.access_token;

    let found = admin
        .call(Method::GET, "/prefabs/search", Some(&token), None)
        .await;

    assert_eq!(found.status, StatusCode::FORBIDDEN);
    assert_eq!(found.json["params"]["permission"], "prefabs.read");
}
