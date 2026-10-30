//! Conversion d'un egg Pterodactyl dans le Studio (ADR-0027) : aperçu sans rien enregistrer,
//! brouillon audité, refus de ce qui n'est pas un egg, permission.

use axum::http::{Method, StatusCode};
use kuro_api::iam::domain::Permission;
use serde_json::{Value, json};

use crate::support::platform::Platform;

/// Egg d'exemple, aussi utilisé par les tests du MCP.
pub(crate) fn sample_egg() -> String {
    egg()
}

fn egg() -> String {
    json!({
        "meta": { "version": "PTDL_v2" },
        "name": "Vanilla Server",
        "author": "someone@example.com",
        "description": "A test egg.",
        "docker_images": { "Java 21": "ghcr.io/pterodactyl/yolks:java_21" },
        "startup": "java -jar {{SERVER_JARFILE}}",
        "config": {
            "files": "{\"server.properties\": {\"parser\": \"properties\", \"find\": {\"server-port\": \"{{server.build.default.port}}\"}}}",
            "startup": "{\"done\": \"Done (\"}",
            "logs": "{}",
            "stop": "stop"
        },
        "scripts": { "installation": {
            "script": "#!/bin/bash\napt-get install -y curl\ncd /mnt/server\n",
            "container": "ghcr.io/pterodactyl/installers:debian",
            "entrypoint": "bash"
        } },
        "variables": [{
            "name": "Server Jar File", "description": "", "env_variable": "SERVER_JARFILE",
            "default_value": "server.jar", "user_viewable": true, "user_editable": true,
            "rules": "required|string|max:64"
        }]
    })
    .to_string()
}

fn refused(notes: &Value, path: &str) -> bool {
    notes
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note["path"] == path && note["kind"] == "refused")
}

#[tokio::test]
async fn an_egg_is_previewed_then_turned_into_a_draft() {
    let platform = Platform::start().await;

    let preview = platform
        .call(Method::POST, "/studio/egg", Some(json!({ "text": egg() })))
        .await;

    assert_eq!(preview.status, StatusCode::OK, "{}", preview.json);
    assert_eq!(preview.json["id"], "vanilla-server");
    assert!(
        refused(&preview.json["notes"], "scripts.installation.script"),
        "{}",
        preview.json["notes"]
    );
    let manifest = preview.json["files"]["prefab.yaml"].as_str().unwrap();
    assert!(
        manifest.contains("STARTUP: \"java -jar ${SERVER_JARFILE}\""),
        "{manifest}"
    );
    assert!(!manifest.contains("someone@example.com"));
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
            Some(json!({ "egg": { "text": egg(), "id": "vanilla" } })),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json);
    assert_eq!(created.json["prefab_id"], "vanilla");
    let event = platform.admin.last_audit("prefab_draft.created").await;
    assert_eq!(event.details["egg"]["source"], "pterodactyl-egg");
    assert_eq!(event.details["egg"]["refused"], 1);
    assert_eq!(event.details["compose"], Value::Null);
}

#[tokio::test]
async fn a_file_that_is_not_an_egg_is_refused() {
    let platform = Platform::start().await;

    let refused = platform
        .call(
            Method::POST,
            "/studio/egg",
            Some(json!({ "text": "{\"name\": \"x\"}" })),
        )
        .await;

    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(refused.json["code"], "invalid_prefab");
    assert_eq!(
        refused.json["params"]["reason"],
        "not a Pterodactyl egg: meta.version is not PTDL_v1 or PTDL_v2"
    );
}

#[tokio::test]
async fn converting_an_egg_requires_the_manage_permission() {
    let platform = Platform::start().await;
    let admin = &platform.admin;
    let account = admin
        .account("reader@example.com", &[Permission::PrefabsRead])
        .await;
    let token = admin.login(&account).await.access_token;

    let preview = admin
        .call(
            Method::POST,
            "/studio/egg",
            Some(&token),
            Some(json!({ "text": egg() })),
        )
        .await;
    let draft = admin
        .call(
            Method::POST,
            "/studio/drafts",
            Some(&token),
            Some(json!({ "egg": { "text": egg() } })),
        )
        .await;

    assert_eq!(preview.status, StatusCode::FORBIDDEN);
    assert_eq!(draft.status, StatusCode::FORBIDDEN);
    assert_eq!(preview.json["params"]["permission"], "prefabs.manage");
}
