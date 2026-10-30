//! Prefab Studio : brouillons (vierge ou copie d'une version), enregistrement même invalide avec
//! sa validation, bornes des fichiers, images, publication par le chemin de l'import,
//! permissions et audit.

use std::io::Write;
use std::path::Path;

use axum::http::{Method, StatusCode};
use kuro_api::iam::domain::Permission;
use serde_json::{Value, json};

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
