//! Mise à jour d'un serveur vers une version plus récente de son prefab (ADR-0024) : aperçu,
//! réglages migrés, sauvegarde attendue, blocages, permissions, mise à jour groupée.

use std::io::Write;
use std::path::Path;

use axum::http::{Method, StatusCode};
use kuro_api::iam::domain::Permission;
use serde_json::{Value, json};
use zip::write::SimpleFileOptions;

use crate::support::platform::{FakeDaemon, Platform};

/// Archive de `minecraft-java` en `version`, où `motd` s'appelle `welcome` (migration depuis
/// les versions antérieures), avec une autre image ; `edit` retouche encore le manifest.
fn minecraft(version: &str, edit: impl Fn(String) -> String) -> Vec<u8> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../prefabs/minecraft-java");
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default();
    // Fins de ligne normalisées : un checkout Windows du sous-module peut être en CRLF.
    let manifest = std::fs::read_to_string(dir.join("prefab.yaml"))
        .unwrap()
        .replace("\r\n", "\n")
        .replace("version: 1.4.0", &format!("version: {version}"))
        .replace("motd", "welcome")
        .replace("2026.10.1-java25", "2026.11.1-java25")
        .replace(
            "lifecycle:\n",
            "migrations:\n  - { from: \"<1.5.0\", rename: { motd: welcome } }\nlifecycle:\n",
        );
    writer.start_file("prefab.yaml", options).unwrap();
    writer.write_all(edit(manifest).as_bytes()).unwrap();
    for lang in ["en", "fr"] {
        let text = std::fs::read_to_string(dir.join(format!("locales/{lang}.json")))
            .unwrap()
            .replace("motd", "welcome");
        writer
            .start_file(format!("locales/{lang}.json"), options)
            .unwrap();
        writer.write_all(text.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

async fn import(platform: &Platform, bytes: Vec<u8>) {
    let imported = platform
        .admin
        .call_bytes(
            Method::POST,
            "/prefabs/import",
            &platform.token,
            bytes,
            true,
        )
        .await;
    assert_eq!(imported.status, StatusCode::CREATED, "{}", imported.json);
}

struct Setup {
    platform: Platform,
    daemon: FakeDaemon,
    id: String,
}

/// Serveur `minecraft-java` 1.4.0 en marche, et une version 1.5.0 importée.
async fn setup() -> Setup {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let (server, _) = platform
        .deploy_minecraft(
            &mut daemon,
            json!({ "motd": "Bienvenue", "difficulty": "hard" }),
        )
        .await;
    let id = server["id"].as_str().unwrap().to_owned();
    daemon.report(&id, "running").await;
    platform.wait_for_state(&id, "running").await;
    import(&platform, minecraft("1.5.0", |m| m)).await;
    Setup {
        platform,
        daemon,
        id,
    }
}

#[tokio::test]
async fn the_preview_shows_migrated_settings_and_changed_images() {
    let setup = setup().await;
    let platform = &setup.platform;
    let available = platform
        .call(
            Method::GET,
            &format!("/servers/{}/upgrades", setup.id),
            None,
        )
        .await;
    assert_eq!(
        available.json,
        json!({ "current": "1.4.0", "available": ["1.5.0"] })
    );

    let preview = platform
        .call(
            Method::GET,
            &format!("/servers/{}/upgrade?to=1.5.0", setup.id),
            None,
        )
        .await;
    assert_eq!(preview.status, StatusCode::OK, "{}", preview.json);
    assert_eq!(
        preview.json["renamed"],
        json!([{ "from": "motd", "to": "welcome" }])
    );
    assert_eq!(preview.json["blockers"], json!([]));
    assert_eq!(preview.json["reinstall"], false);
    assert_eq!(
        preview.json["images"],
        json!([{
            "service": "server",
            "from": "docker.io/itzg/minecraft-server:2026.10.1-java25",
            "to": "docker.io/itzg/minecraft-server:2026.11.1-java25",
        }])
    );

    // Ni la même version, ni une plus ancienne.
    let same = platform
        .call(
            Method::GET,
            &format!("/servers/{}/upgrade?to=1.4.0", setup.id),
            None,
        )
        .await;
    assert_eq!(same.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(same.json["code"], "upgrade_not_newer");
}

#[tokio::test]
async fn an_upgrade_migrates_the_settings_and_redeploys() {
    let mut setup = setup().await;
    let path = format!("/servers/{}/upgrade", setup.id);
    let (started, command) = tokio::join!(
        setup.platform.call(
            Method::POST,
            &path,
            Some(json!({ "to": "1.5.0", "backup": false })),
        ),
        setup.daemon.accept(),
    );
    assert_eq!(started.status, StatusCode::ACCEPTED, "{}", started.json);
    assert_eq!(command["type"], "server.deploy");
    assert!(command["payload"]["install"].is_null());
    assert_eq!(
        command["payload"]["services"][0]["image"],
        "docker.io/itzg/minecraft-server:2026.11.1-java25"
    );

    let server = setup
        .platform
        .call(Method::GET, &format!("/servers/{}", setup.id), None)
        .await;
    assert_eq!(server.json["prefab"]["version"], "1.5.0");
    assert_eq!(server.json["settings"]["welcome"], "Bienvenue");
    assert!(server.json["settings"].get("motd").is_none());
    let event = setup.platform.admin.last_audit("server.upgraded").await;
    assert_eq!(event.outcome, "success");
    assert_eq!(event.details["from"], "1.4.0");
    assert_eq!(event.details["to"], "1.5.0");
    assert!(!event.details.to_string().contains("Bienvenue"));
}

#[tokio::test]
async fn the_backup_is_awaited_before_anything_changes() {
    let mut setup = setup().await;
    let started = setup
        .platform
        .call(
            Method::POST,
            &format!("/servers/{}/upgrade", setup.id),
            Some(json!({ "to": "1.5.0", "backup": true })),
        )
        .await;
    assert_eq!(started.status, StatusCode::ACCEPTED, "{}", started.json);

    let request = setup.daemon.accept_backup().await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let before = setup
        .platform
        .call(Method::GET, &format!("/servers/{}", setup.id), None)
        .await;
    assert_eq!(before.json["prefab"]["version"], "1.4.0");

    setup
        .daemon
        .event(
            "backup.done",
            json!({
                "server_id": setup.id,
                "backup_id": request["payload"]["backup_id"],
                "status": "ok",
                "size": 10,
                "sha256": "324fb9b3d3a59324cb5d96154f4fbc41c8f2d926f9bb2da1702f0230bd1b88d6",
            }),
        )
        .await;
    let command = setup.daemon.accept().await;
    assert_eq!(command["type"], "server.deploy");
    let event = setup.platform.admin.last_audit("server.upgraded").await;
    assert_eq!(event.details["backup"], true);
}

#[tokio::test]
async fn a_setting_no_longer_valid_blocks_the_upgrade() {
    let setup = setup().await;
    // 1.6.0 retire la difficulté `hard`, celle du serveur.
    import(
        &setup.platform,
        minecraft("1.6.0", |m| {
            m.replace("[peaceful, easy, normal, hard]", "[peaceful, easy, normal]")
        }),
    )
    .await;
    let preview = setup
        .platform
        .call(
            Method::GET,
            &format!("/servers/{}/upgrade?to=1.6.0", setup.id),
            None,
        )
        .await;
    assert_eq!(
        preview.json["blockers"],
        json!([{ "kind": "setting", "field": "difficulty", "reason": "options" }])
    );
    let refused = setup
        .platform
        .call(
            Method::POST,
            &format!("/servers/{}/upgrade", setup.id),
            Some(json!({ "to": "1.6.0" })),
        )
        .await;
    assert_eq!(refused.status, StatusCode::CONFLICT);
    assert_eq!(refused.json["code"], "upgrade_blocked");
}

#[tokio::test]
async fn a_draft_version_is_an_upgrade_on_a_test_node_only() {
    let setup = setup().await;
    import(
        &setup.platform,
        minecraft("1.6.0", |m| {
            m.replace(
                "lifecycle:\n",
                "x-status: \"draft: new runtime image\"\nlifecycle:\n",
            )
        }),
    )
    .await;
    let path = format!("/servers/{}/upgrade?to=1.6.0", setup.id);

    let preview = setup.platform.call(Method::GET, &path, None).await;
    assert_eq!(
        preview.json["blockers"],
        json!([{ "kind": "not_deployable" }])
    );

    // Sur un nœud de test, le brouillon se met au point comme une version publiée.
    let marked = setup
        .platform
        .call(
            Method::PATCH,
            &format!("/nodes/{}", setup.daemon.node.id.as_uuid()),
            Some(json!({ "trials": true })),
        )
        .await;
    assert_eq!(marked.status, StatusCode::OK, "{}", marked.json);
    let preview = setup.platform.call(Method::GET, &path, None).await;
    assert_eq!(preview.json["blockers"], json!([]), "{}", preview.json);
}

#[tokio::test]
async fn upgrading_needs_its_permission_and_the_backup_one() {
    let setup = setup().await;
    let admin = &setup.platform.admin;
    let call = |permissions: &'static [Permission], backup: bool| {
        let id = setup.id.clone();
        async move {
            let account = admin
                .account(
                    &format!("u{}@example.com", uuid::Uuid::now_v7().simple()),
                    permissions,
                )
                .await;
            let token = admin.login(&account).await.access_token;
            admin
                .call(
                    Method::POST,
                    &format!("/servers/{id}/upgrade"),
                    Some(&token),
                    Some(json!({ "to": "1.5.0", "backup": backup })),
                )
                .await
        }
    };
    let without = call(&[Permission::ServersRead, Permission::ServersUpdate], false).await;
    assert_eq!(without.status, StatusCode::FORBIDDEN);
    assert_eq!(without.json["params"]["permission"], "servers.upgrade");
    let no_backup_right = call(&[Permission::ServersUpgrade], true).await;
    assert_eq!(no_backup_right.status, StatusCode::FORBIDDEN);
    assert_eq!(
        no_backup_right.json["params"]["permission"],
        "servers.backups.create"
    );
}

#[tokio::test]
async fn all_older_servers_of_a_prefab_are_upgraded_together() {
    let mut setup = setup().await;
    let candidates = setup
        .platform
        .call(Method::GET, "/prefabs/minecraft-java/1.5.0/upgrades", None)
        .await;
    assert_eq!(candidates.status, StatusCode::OK, "{}", candidates.json);
    let list = candidates.json.as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["server_id"], setup.id);
    assert_eq!(list[0]["preview"]["blockers"], json!([]));

    let unknown = uuid::Uuid::now_v7();
    let (started, command) = tokio::join!(
        setup.platform.call(
            Method::POST,
            "/servers/upgrade",
            Some(json!({ "servers": [setup.id, unknown], "to": "1.5.0" })),
        ),
        setup.daemon.accept(),
    );
    assert_eq!(started.status, StatusCode::ACCEPTED, "{}", started.json);
    let results: &Vec<Value> = started.json.as_array().unwrap();
    assert_eq!(results[0]["started"], true);
    assert_eq!(
        results[1],
        json!({ "server_id": unknown, "started": false, "error": "not_found" })
    );
    assert_eq!(command["type"], "server.deploy");
}
