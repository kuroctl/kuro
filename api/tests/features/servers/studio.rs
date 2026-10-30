//! Prefab Studio : brouillons (vierge ou copie d'une version), enregistrement même invalide avec
//! sa validation, bornes des fichiers, images, publication par le chemin de l'import,
//! permissions et audit.

use std::io::Write;
use std::path::Path;

use axum::http::{Method, StatusCode};
use kuro_api::content::steam_store::StoreGame;
use kuro_api::content::steamgriddb::GridImage;
use kuro_api::iam::domain::Permission;
use kuro_prefab::assets::AssetKind;
use kuro_prefab::files::PrefabFiles;
use serde_json::{Value, json};

use crate::support::content::GRID_KEY;
use crate::support::platform::Platform;

const PNG: &[u8] = &[
    0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0, 0, 0, 0,
];

async fn blank(platform: &Platform) -> Value {
    let created = platform
        .call(Method::POST, "/studio/drafts", Some(json!({})))
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json);
    created.json
}

fn path(draft: &Value) -> String {
    format!("/studio/drafts/{}", draft["id"].as_str().unwrap())
}

/// Fichiers du brouillon, `prefab.yaml` modifié par `edit`.
fn with_manifest(draft: &Value, edit: impl Fn(&str) -> String) -> Value {
    let mut files = draft["files"].clone();
    let manifest = files["prefab.yaml"].as_str().unwrap().to_owned();
    files["prefab.yaml"] = json!(edit(&manifest));
    json!({ "files": files })
}

#[tokio::test]
async fn a_blank_draft_is_a_valid_example_and_is_audited() {
    let platform = Platform::start().await;

    let draft = blank(&platform).await;

    assert_eq!(draft["prefab_id"], "my-prefab");
    assert_eq!(draft["prefab_version"], "0.1.0");
    assert_eq!(draft["check"]["valid"], true, "{}", draft["check"]);
    assert!(draft["files"]["locales/fr.json"].is_string());
    let event = platform.admin.last_audit("prefab_draft.created").await;
    assert_eq!(event.actor_kind, "admin");
    assert_eq!(event.details["draft"], draft["id"]);

    let list = platform.call(Method::GET, "/studio/drafts", None).await;
    assert_eq!(list.json[0]["id"], draft["id"]);
}

#[tokio::test]
async fn a_draft_is_saved_even_when_invalid_and_says_why() {
    let platform = Platform::start().await;
    let draft = blank(&platform).await;

    // Image sans tag : problème situé dans le manifest.
    let saved = platform
        .call(
            Method::PUT,
            &path(&draft),
            Some(with_manifest(&draft, |text| {
                text.replace("debian:bookworm-20261020-slim", "debian")
            })),
        )
        .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.json);
    assert_eq!(saved.json["check"]["valid"], false);
    let issues = saved.json["check"]["issues"].as_array().unwrap();
    assert!(
        issues
            .iter()
            .any(|issue| issue["path"] == "services.server.image"),
        "{issues:?}"
    );
    let event = platform.admin.last_audit("prefab_draft.updated").await;
    assert_eq!(event.details["valid"], false);

    // YAML illisible : enregistré quand même, refus global.
    let broken = platform
        .call(
            Method::PUT,
            &path(&draft),
            Some(with_manifest(&draft, |text| format!("{text}\n  : : [\n"))),
        )
        .await;
    assert_eq!(broken.status, StatusCode::OK, "{}", broken.json);
    assert!(
        broken.json["check"]["reason"]
            .as_str()
            .unwrap()
            .contains("prefab.yaml")
    );
    let reread = platform.call(Method::GET, &path(&draft), None).await;
    assert!(
        reread.json["files"]["prefab.yaml"]
            .as_str()
            .unwrap()
            .ends_with(": : [\n")
    );
}

#[tokio::test]
async fn files_outside_the_prefab_or_too_large_are_refused_before_any_write() {
    let platform = Platform::start().await;
    let draft = blank(&platform).await;

    for (name, content) in [
        ("../escape.json", "{}".to_owned()),
        ("/etc/passwd", "x".to_owned()),
        ("locales/huge.json", "x".repeat(2 * 1024 * 1024 + 1)),
    ] {
        let mut files = draft["files"].clone();
        files[name] = json!(content);
        let refused = platform
            .call(Method::PUT, &path(&draft), Some(json!({ "files": files })))
            .await;
        assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY, "{name}");
        assert_eq!(refused.json["code"], "invalid_prefab");
    }
    let mut files = draft["files"].clone();
    files.as_object_mut().unwrap().remove("prefab.yaml");
    let refused = platform
        .call(Method::PUT, &path(&draft), Some(json!({ "files": files })))
        .await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);

    let unchanged = platform.call(Method::GET, &path(&draft), None).await;
    assert_eq!(unchanged.json["files"], draft["files"]);
}

#[tokio::test]
async fn validating_writes_nothing() {
    let platform = Platform::start().await;
    let draft = blank(&platform).await;

    let checked = platform
        .call(
            Method::POST,
            "/studio/validate",
            Some(with_manifest(&draft, |text| {
                text.replace("id: my-prefab", "id: Not Valid")
            })),
        )
        .await;

    assert_eq!(checked.status, StatusCode::OK, "{}", checked.json);
    assert_eq!(checked.json["valid"], false);
    let unchanged = platform.call(Method::GET, &path(&draft), None).await;
    assert_eq!(unchanged.json["files"], draft["files"]);
}

#[tokio::test]
async fn copying_an_official_version_keeps_its_files_bumps_the_version_and_takes_its_images() {
    let platform = Platform::start().await;

    let copied = platform
        .call(
            Method::POST,
            "/studio/drafts",
            Some(json!({ "from": { "id": "terraria", "version": "1.0.0" } })),
        )
        .await;

    assert_eq!(copied.status, StatusCode::CREATED, "{}", copied.json);
    let manifest = copied.json["files"]["prefab.yaml"].as_str().unwrap();
    // Fichier d'origine : commentaires compris.
    assert!(manifest.contains("# Terraria"), "{manifest}");
    assert!(manifest.contains("\nversion: 1.0.1\n"), "{manifest}");
    assert_eq!(copied.json["prefab_version"], "1.0.1");
    assert_eq!(
        (
            copied.json["base_id"].clone(),
            copied.json["base_version"].clone()
        ),
        (json!("terraria"), json!("1.0.0"))
    );
    assert_eq!(copied.json["assets"], json!(["cover", "icon", "thumbnail"]));
    assert_eq!(
        copied.json["check"]["valid"], true,
        "{}",
        copied.json["check"]
    );
    let event = platform.admin.last_audit("prefab_draft.created").await;
    assert_eq!(event.details["from"], "terraria@1.0.0");
}

/// Archive du prefab officiel `terraria` en version `version`.
fn terraria_archive(version: &str) -> Vec<u8> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../prefabs/terraria");
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    let manifest = std::fs::read_to_string(dir.join("prefab.yaml"))
        .unwrap()
        .replace("version: 1.0.0", &format!("version: {version}"));
    writer.start_file("prefab.yaml", options).unwrap();
    writer.write_all(manifest.as_bytes()).unwrap();
    for lang in ["en", "fr"] {
        writer
            .start_file(format!("locales/{lang}.json"), options)
            .unwrap();
        writer
            .write_all(&std::fs::read(dir.join(format!("locales/{lang}.json"))).unwrap())
            .unwrap();
    }
    writer.finish().unwrap().into_inner()
}

#[tokio::test]
async fn copying_an_imported_version_rewrites_its_manifest_without_empty_fields() {
    let platform = Platform::start().await;
    let imported = platform
        .admin
        .call_bytes(
            Method::POST,
            "/prefabs/import",
            &platform.token,
            terraria_archive("2.0.0"),
            true,
        )
        .await;
    assert_eq!(imported.status, StatusCode::CREATED, "{}", imported.json);

    let copied = platform
        .call(
            Method::POST,
            "/studio/drafts",
            Some(json!({ "from": { "id": "terraria", "version": "2.0.0" } })),
        )
        .await;

    assert_eq!(copied.status, StatusCode::CREATED, "{}", copied.json);
    let manifest = copied.json["files"]["prefab.yaml"].as_str().unwrap();
    assert!(!manifest.contains("null"), "{manifest}");
    assert_eq!(copied.json["prefab_version"], "2.0.1");
    assert_eq!(
        copied.json["check"]["valid"], true,
        "{}",
        copied.json["check"]
    );
    assert!(
        copied.json["files"]["locales/fr.json"]
            .as_str()
            .unwrap()
            .contains("Terraria")
    );
}

#[tokio::test]
async fn images_are_checked_by_content() {
    let platform = Platform::start().await;
    let draft = blank(&platform).await;
    let icon = format!("{}/assets/icon", path(&draft));

    let svg = platform
        .admin
        .call_bytes(
            Method::PUT,
            &icon,
            &platform.token,
            b"<svg/>".to_vec(),
            true,
        )
        .await;
    assert_eq!(svg.status, StatusCode::UNPROCESSABLE_ENTITY);

    let stored = platform
        .admin
        .call_bytes(Method::PUT, &icon, &platform.token, PNG.to_vec(), true)
        .await;
    assert_eq!(stored.status, StatusCode::NO_CONTENT);
    let served = platform.call(Method::GET, &icon, None).await;
    assert_eq!(served.content_type.as_deref(), Some("image/png"));
    let event = platform.admin.last_audit("prefab_draft.updated").await;
    assert_eq!(event.details["asset"], "icon");

    let removed = platform.call(Method::DELETE, &icon, None).await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    let gone = platform.call(Method::GET, &icon, None).await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn publishing_imports_the_draft_with_its_images_and_removes_it() {
    let platform = Platform::start().await;
    let draft = blank(&platform).await;
    platform
        .admin
        .call_bytes(
            Method::PUT,
            &format!("{}/assets/icon", path(&draft)),
            &platform.token,
            PNG.to_vec(),
            true,
        )
        .await;

    let published = platform
        .call(Method::POST, &format!("{}/publish", path(&draft)), None)
        .await;

    assert_eq!(published.status, StatusCode::CREATED, "{}", published.json);
    assert_eq!(published.json["id"], "my-prefab");
    assert_eq!(published.json["source"], "uploaded");
    assert_eq!(published.json["assets"], json!(["icon"]));
    let imported = platform.admin.last_audit("prefab.imported").await;
    assert_eq!(imported.details["draft"], draft["id"]);
    let event = platform.admin.last_audit("prefab_draft.published").await;
    assert_eq!(event.details["prefab"], "my-prefab");
    let gone = platform.call(Method::GET, &path(&draft), None).await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);

    // Même version une seconde fois : refusée, le brouillon reste.
    let again = blank(&platform).await;
    let conflict = platform
        .call(Method::POST, &format!("{}/publish", path(&again)), None)
        .await;
    assert_eq!(conflict.status, StatusCode::CONFLICT);
    assert_eq!(conflict.json["code"], "prefab_exists");
    let kept = platform.call(Method::GET, &path(&again), None).await;
    assert_eq!(kept.status, StatusCode::OK);
}

#[tokio::test]
async fn an_invalid_draft_is_not_published() {
    let platform = Platform::start().await;
    let draft = blank(&platform).await;
    platform
        .call(
            Method::PUT,
            &path(&draft),
            Some(with_manifest(&draft, |text| {
                text.replace("id: my-prefab", "id: Not Valid")
            })),
        )
        .await;

    let refused = platform
        .call(Method::POST, &format!("{}/publish", path(&draft)), None)
        .await;

    assert_eq!(
        refused.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        refused.json
    );
    assert_eq!(refused.json["code"], "invalid_prefab");
    let event = platform.admin.last_audit("prefab.imported").await;
    assert_eq!(event.outcome, "failure");
    assert_eq!(event.details["draft"], draft["id"]);
    let kept = platform.call(Method::GET, &path(&draft), None).await;
    assert_eq!(kept.status, StatusCode::OK);
}

#[tokio::test]
async fn the_studio_requires_the_manage_permission_but_the_schema_is_readable() {
    let platform = Platform::start().await;
    let draft = blank(&platform).await;
    let admin = &platform.admin;
    let account = admin
        .account("reader@example.com", &[Permission::PrefabsRead])
        .await;
    let token = admin.login(&account).await.access_token;

    for (method, path, body) in [
        (Method::GET, "/studio/drafts".to_owned(), None),
        (Method::POST, "/studio/drafts".to_owned(), Some(json!({}))),
        (Method::GET, path(&draft), None),
        (Method::DELETE, path(&draft), None),
        (Method::POST, format!("{}/publish", path(&draft)), None),
        (Method::POST, format!("{}/assets/steam", path(&draft)), None),
    ] {
        let refused = admin.call(method.clone(), &path, Some(&token), body).await;
        assert_eq!(refused.status, StatusCode::FORBIDDEN, "{method} {path}");
        assert_eq!(refused.json["params"]["permission"], "prefabs.manage");
    }
    let schema = admin
        .call(Method::GET, "/studio/schema", Some(&token), None)
        .await;
    assert_eq!(schema.status, StatusCode::OK);
    assert_eq!(schema.json["title"], "Kuro prefab manifest (v1)");
}

#[tokio::test]
async fn deleting_a_draft_is_audited() {
    let platform = Platform::start().await;
    let draft = blank(&platform).await;

    let deleted = platform.call(Method::DELETE, &path(&draft), None).await;

    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    let event = platform.admin.last_audit("prefab_draft.deleted").await;
    assert_eq!(event.details["draft"], draft["id"]);
    let gone = platform.call(Method::DELETE, &path(&draft), None).await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
}

const COMPOSE: &str = r#"
services:
  web:
    image: nginx:1.27-alpine
    privileged: true
    ports: ["8080:80"]
    environment:
      API_TOKEN: ${API_TOKEN}
"#;

#[tokio::test]
async fn a_compose_file_is_previewed_then_turned_into_a_draft() {
    let platform = Platform::start().await;

    let preview = platform
        .call(
            Method::POST,
            "/studio/compose",
            Some(json!({ "text": COMPOSE, "id": "my-nginx" })),
        )
        .await;

    assert_eq!(preview.status, StatusCode::OK, "{}", preview.json);
    assert_eq!(preview.json["id"], "my-nginx");
    let notes = preview.json["notes"].as_array().unwrap();
    assert!(
        notes
            .iter()
            .any(|note| note["path"] == "services.web.privileged" && note["kind"] == "refused"),
        "{notes:?}"
    );
    assert!(
        preview.json["files"]["prefab.yaml"]
            .as_str()
            .unwrap()
            .contains("api_token: { type: secret")
    );
    assert_eq!(
        preview.json["check"]["valid"], true,
        "{}",
        preview.json["check"]
    );
    // Un aperçu n'enregistre rien.
    let drafts = platform.call(Method::GET, "/studio/drafts", None).await;
    assert_eq!(drafts.json, json!([]));

    let created = platform
        .call(
            Method::POST,
            "/studio/drafts",
            Some(json!({ "compose": { "text": COMPOSE, "id": "my-nginx" } })),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json);
    assert_eq!(created.json["prefab_id"], "my-nginx");
    let event = platform.admin.last_audit("prefab_draft.created").await;
    assert_eq!(event.details["compose"]["source"], "docker-compose");
    assert_eq!(event.details["compose"]["refused"], 1);
}

#[tokio::test]
async fn a_file_that_is_not_a_compose_is_refused() {
    let platform = Platform::start().await;

    let refused = platform
        .call(
            Method::POST,
            "/studio/compose",
            Some(json!({ "text": "hello: world" })),
        )
        .await;

    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(refused.json["code"], "invalid_prefab");
    assert_eq!(refused.json["params"]["reason"], "no service to convert");
}

#[tokio::test]
async fn converting_a_compose_requires_the_manage_permission() {
    let platform = Platform::start().await;
    let admin = &platform.admin;
    let account = admin
        .account("reader@example.com", &[Permission::PrefabsRead])
        .await;
    let token = admin.login(&account).await.access_token;

    let refused = admin
        .call(
            Method::POST,
            "/studio/compose",
            Some(&token),
            Some(json!({ "text": COMPOSE })),
        )
        .await;

    assert_eq!(refused.status, StatusCode::FORBIDDEN);
}

/// Brouillon vierge enregistré avec `steam: { app }` dans son manifest.
async fn steam_draft(platform: &Platform, app: u32) -> Value {
    let draft = blank(platform).await;
    let body = with_manifest(&draft, |manifest| {
        manifest.replace(
            "category: game
",
            &format!(
                "category: game
steam:
  app: {app}
"
            ),
        )
    });
    let saved = platform.call(Method::PUT, &path(&draft), Some(body)).await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.json);
    assert_eq!(
        saved.json["check"]["valid"], true,
        "{}",
        saved.json["check"]
    );
    draft
}

/// CS2 sur le magasin simulé : jaquette et petite icône publiées ; ni bannière, ni grande
/// icône (premier choix), ni logo.
fn counter_strike(platform: &Platform) {
    let store = &platform.admin.steam_store;
    let paths = |list: &[&str]| list.iter().map(|p| (*p).to_owned()).collect::<Vec<_>>();
    store.games.lock().unwrap().insert(
        730,
        StoreGame {
            name: "Counter-Strike 2".to_owned(),
            images: vec![
                (AssetKind::Thumbnail, paths(&["capsule.jpg"])),
                (AssetKind::Cover, paths(&["hero.jpg"])),
                (
                    AssetKind::Icon,
                    paths(&["icon_full.jpg", "icon.jpg", "logo.png"]),
                ),
            ],
        },
    );
    let mut images = store.images.lock().unwrap();
    images.insert("capsule.jpg".to_owned(), PNG.to_vec());
    images.insert("icon.jpg".to_owned(), PNG.to_vec());
}

#[tokio::test]
async fn the_images_of_the_declared_steam_game_are_imported_and_audited() {
    let platform = Platform::start().await;
    counter_strike(&platform);
    let draft = steam_draft(&platform, 730).await;

    let imported = platform
        .call(
            Method::POST,
            &format!("{}/assets/steam", path(&draft)),
            None,
        )
        .await;

    assert_eq!(imported.status, StatusCode::OK, "{}", imported.json);
    assert_eq!(imported.json["app"], 730);
    assert_eq!(imported.json["name"], "Counter-Strike 2");
    assert_eq!(imported.json["imported"], json!(["icon", "thumbnail"]));
    assert_eq!(imported.json["missing"], json!(["cover"]));
    let icon = platform
        .call(Method::GET, &format!("{}/assets/icon", path(&draft)), None)
        .await;
    assert_eq!(icon.content_type.as_deref(), Some("image/png"));
    let refreshed = platform.call(Method::GET, &path(&draft), None).await;
    assert_eq!(refreshed.json["assets"], json!(["icon", "thumbnail"]));
    // Grande icône absente : la petite suffit, le logo n'est pas demandé.
    let fetched = platform.admin.steam_store.fetched.lock().unwrap().clone();
    assert!(fetched.contains(&"icon.jpg".to_owned()), "{fetched:?}");
    assert!(!fetched.contains(&"logo.png".to_owned()), "{fetched:?}");
    let event = platform.admin.last_audit("prefab_draft.updated").await;
    assert_eq!(event.details["steam_app"], 730);
    assert_eq!(event.details["draft"], draft["id"]);
}

#[tokio::test]
async fn steam_images_need_a_game_declared_and_known_by_the_store() {
    let platform = Platform::start().await;
    counter_strike(&platform);
    let undeclared = blank(&platform).await;
    let unknown = steam_draft(&platform, 896_660).await;
    let known = steam_draft(&platform, 730).await;
    let import = |draft: &Value| format!("{}/assets/steam", path(draft));

    let missing = platform
        .call(Method::POST, &import(&undeclared), None)
        .await;
    assert_eq!(missing.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(missing.json["code"], "steam_game_missing");

    let not_found = platform.call(Method::POST, &import(&unknown), None).await;
    assert_eq!(not_found.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(not_found.json["code"], "steam_game_not_found");

    *platform.admin.steam_store.unavailable.lock().unwrap() = true;
    let down = platform.call(Method::POST, &import(&known), None).await;
    assert_eq!(down.status, StatusCode::BAD_GATEWAY);
    assert_eq!(down.json["code"], "provider_unavailable");
    let untouched = platform.call(Method::GET, &path(&known), None).await;
    assert_eq!(untouched.json["assets"], json!([]));
}

/// Clé SteamGridDB réglée ; une icône de la communauté publiée pour le jeu.
async fn steamgriddb_with_an_icon(platform: &Platform) {
    let refused = platform
        .call(
            Method::PUT,
            "/settings/content-sources/steamgriddb",
            Some(json!({ "api_key": "wrong" })),
        )
        .await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    let saved = platform
        .call(
            Method::PUT,
            "/settings/content-sources/steamgriddb",
            Some(json!({ "api_key": GRID_KEY })),
        )
        .await;
    assert_eq!(saved.status, StatusCode::NO_CONTENT, "{}", saved.json);
    assert_eq!(platform.admin.steam_grid.key().as_deref(), Some(GRID_KEY));
    let url = "https://cdn2.steamgriddb.com/icon/abc.png";
    platform.admin.steam_grid.images.lock().unwrap().insert(
        AssetKind::Icon,
        vec![GridImage {
            id: 11,
            url: url.to_owned(),
            author: Some("artist".to_owned()),
        }],
    );
    platform
        .admin
        .steam_grid
        .files
        .lock()
        .unwrap()
        .insert(url.to_owned(), PNG.to_vec());
}

#[tokio::test]
async fn steamgriddb_images_come_first_and_steam_fills_the_rest() {
    let platform = Platform::start().await;
    counter_strike(&platform);
    steamgriddb_with_an_icon(&platform).await;
    let draft = steam_draft(&platform, 730).await;

    let imported = platform
        .call(
            Method::POST,
            &format!("{}/assets/steam", path(&draft)),
            None,
        )
        .await;

    assert_eq!(imported.status, StatusCode::OK, "{}", imported.json);
    assert_eq!(imported.json["imported"], json!(["icon", "thumbnail"]));
    assert_eq!(
        imported.json["sources"],
        json!({ "icon": "steamgriddb", "thumbnail": "steam" })
    );
    assert_eq!(imported.json["missing"], json!(["cover"]));
    // L'icône de SteamGridDB suffit : celles du magasin ne sont pas demandées.
    let fetched = platform.admin.steam_store.fetched.lock().unwrap().clone();
    assert!(
        !fetched.iter().any(|p| p.starts_with("icon")),
        "{fetched:?}"
    );
    let event = platform.admin.last_audit("prefab_draft.updated").await;
    assert_eq!(event.details["asset"], "icon");
    assert_eq!(event.details["source"], "steamgriddb");
    assert_eq!(event.details["steamgriddb_image"], 11);
    assert_eq!(event.details["author"], "artist");
}

#[tokio::test]
async fn a_steamgriddb_outage_falls_back_to_steam() {
    let platform = Platform::start().await;
    counter_strike(&platform);
    steamgriddb_with_an_icon(&platform).await;
    *platform.admin.steam_grid.unavailable.lock().unwrap() = true;
    let draft = steam_draft(&platform, 730).await;

    let imported = platform
        .call(
            Method::POST,
            &format!("{}/assets/steam", path(&draft)),
            None,
        )
        .await;

    assert_eq!(imported.status, StatusCode::OK, "{}", imported.json);
    assert_eq!(
        imported.json["sources"],
        json!({ "icon": "steam", "thumbnail": "steam" })
    );
}

#[tokio::test]
async fn a_draft_is_exported_as_an_archive_with_its_images_declared() {
    let platform = Platform::start().await;
    let draft = blank(&platform).await;
    platform
        .admin
        .call_bytes(
            Method::PUT,
            &format!("{}/assets/icon", path(&draft)),
            &platform.token,
            PNG.to_vec(),
            true,
        )
        .await;

    let exported = platform
        .call(Method::GET, &format!("{}/export", path(&draft)), None)
        .await;
    assert_eq!(exported.status, StatusCode::OK);
    assert_eq!(exported.content_type.as_deref(), Some("application/zip"));
    assert_eq!(
        exported.headers["content-disposition"],
        "attachment; filename=\"my-prefab-0.1.0.kprefab\""
    );
    // Un dossier de tête `my-prefab/`, retiré à la lecture ; l'icône, non déclarée par l'exemple,
    // l'est devenue : le dossier s'importe avec elle.
    let files = kuro_prefab::files::ArchiveFiles::from_zip(&exported.bytes).unwrap();
    let package = kuro_prefab::Package::load(&files).unwrap();
    assert_eq!(package.manifest.id, "my-prefab");
    assert_eq!(
        package.manifest.assets.icon.as_deref(),
        Some("assets/icon.png")
    );
    let assets = kuro_prefab::assets::load_assets_from(&files, &package.manifest).unwrap();
    assert_eq!(assets.len(), 1);
    assert_eq!(assets[0].kind, AssetKind::Icon);
    // Le brouillon n'a pas bougé : l'export est une lecture.
    let kept = platform.call(Method::GET, &path(&draft), None).await;
    assert_eq!(kept.json["files"], draft["files"]);

    // Réimportable telle quelle.
    let imported = platform
        .admin
        .call_bytes(
            Method::POST,
            "/prefabs/import",
            &platform.token,
            exported.bytes.clone(),
            true,
        )
        .await;
    assert_eq!(imported.status, StatusCode::CREATED, "{}", imported.json);
    assert_eq!(imported.json["assets"], json!(["icon"]));
}

#[tokio::test]
async fn a_catalog_version_is_exported_with_its_original_files_or_regenerated() {
    let platform = Platform::start().await;
    // Version officielle dont le dossier est là : ses fichiers d'origine, commentaires compris.
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../prefabs/minecraft-java");
    let original = std::fs::read_to_string(folder.join("prefab.yaml")).unwrap();
    let version = kuro_prefab::package::parse_manifest(&original)
        .unwrap()
        .version
        .to_string();
    let exported = platform
        .call(
            Method::GET,
            &format!("/prefabs/minecraft-java/{version}/export"),
            None,
        )
        .await;
    assert_eq!(exported.status, StatusCode::OK, "{}", exported.json);
    assert_eq!(
        exported.headers["content-disposition"],
        format!("attachment; filename=\"minecraft-java-{version}.kprefab\"")
    );
    let files = kuro_prefab::files::ArchiveFiles::from_zip(&exported.bytes).unwrap();
    let manifest = files.read("prefab.yaml", 1 << 20).unwrap().unwrap();
    assert_eq!(String::from_utf8(manifest).unwrap(), original);
    assert!(files.read("locales/fr.json", 1 << 20).unwrap().is_some());

    // Version importée : ses fichiers d'origine ne sont pas conservés, le manifest est régénéré.
    let draft = blank(&platform).await;
    let published = platform
        .call(Method::POST, &format!("{}/publish", path(&draft)), None)
        .await;
    assert_eq!(published.status, StatusCode::CREATED, "{}", published.json);
    let exported = platform
        .call(Method::GET, "/prefabs/my-prefab/0.1.0/export", None)
        .await;
    assert_eq!(exported.status, StatusCode::OK);
    let files = kuro_prefab::files::ArchiveFiles::from_zip(&exported.bytes).unwrap();
    let package = kuro_prefab::Package::load(&files).unwrap();
    assert_eq!(package.manifest.id, "my-prefab");
    assert_eq!(package.manifest.version.to_string(), "0.1.0");

    let unknown = platform
        .call(Method::GET, "/prefabs/my-prefab/9.9.9/export", None)
        .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn publishing_to_the_prefabs_directory_writes_the_folder_and_imports_it_as_official() {
    use kuro_api::audit::domain::AuditContext;
    use kuro_api::prefabs::application::{PrefabsDir, PrefabsError};

    let platform = Platform::start().await;
    let draft = blank(&platform).await;
    platform
        .admin
        .call_bytes(
            Method::PUT,
            &format!("{}/assets/icon", path(&draft)),
            &platform.token,
            PNG.to_vec(),
            true,
        )
        .await;
    let draft_id = uuid::Uuid::parse_str(draft["id"].as_str().unwrap()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let prefabs_dir = PrefabsDir {
        dir: dir.path().to_path_buf(),
        publish_to_dir: true,
    };

    let imported = kuro_api::studio::application::publish(
        &platform.admin.pool,
        &AuditContext::cli("test"),
        &prefabs_dir,
        draft_id,
    )
    .await
    .unwrap();
    assert_eq!(
        (imported.id.as_str(), imported.version.as_str()),
        ("my-prefab", "0.1.0")
    );

    // Le dossier `<id>/` porte les fichiers du brouillon et l'icône, déclarée.
    let folder = dir.path().join("my-prefab");
    let manifest = std::fs::read_to_string(folder.join("prefab.yaml")).unwrap();
    assert!(
        manifest.contains("\nassets:\n  icon: assets/icon.png\n"),
        "{manifest}"
    );
    assert!(folder.join("locales/en.json").is_file());
    assert!(folder.join("assets/icon.png").is_file());
    // Au catalogue en `official`, comme au démarrage ; le brouillon a disparu.
    let detail = platform
        .call(Method::GET, "/prefabs/my-prefab/0.1.0", None)
        .await;
    assert_eq!(detail.status, StatusCode::OK, "{}", detail.json);
    assert_eq!(detail.json["source"], "official");
    assert_eq!(detail.json["assets"], json!(["icon"]));
    let event = platform.admin.last_audit("prefab_draft.published").await;
    assert_eq!(event.details["exported_to"], "my-prefab");
    let gone = platform.call(Method::GET, &path(&draft), None).await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);

    // La même version une seconde fois : refusée avant toute écriture, le dossier est intact.
    let again = blank(&platform).await;
    let again_id = uuid::Uuid::parse_str(again["id"].as_str().unwrap()).unwrap();
    let refused = kuro_api::studio::application::publish(
        &platform.admin.pool,
        &AuditContext::cli("test"),
        &prefabs_dir,
        again_id,
    )
    .await;
    assert!(matches!(refused, Err(PrefabsError::Exists)), "{refused:?}");
    assert_eq!(
        std::fs::read_to_string(folder.join("prefab.yaml")).unwrap(),
        manifest
    );
    let kept = platform.call(Method::GET, &path(&again), None).await;
    assert_eq!(kept.status, StatusCode::OK);
}
