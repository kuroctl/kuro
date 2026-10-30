//! CurseForge (ADR-0029) : clé d'API de l'installation (vérifiée, chiffrée, jamais rendue),
//! emplacement à plusieurs sources, fichiers vérifiés par SHA-1, distribution refusée par
//! l'auteur, mises à jour, et mode `reference` (le jeu télécharge lui-même ses mods).

use std::io::Write;
use std::path::Path;

use axum::http::{Method, StatusCode};
use kuro_api::iam::domain::Permission;
use serde_json::{Value, json};
use zip::write::SimpleFileOptions;

use crate::support::admin::PASSWORD;
use crate::support::content::{CURSEFORGE_KEY, curseforge_version};
use crate::support::platform::{FakeDaemon, Platform};

const GAME: &str = "1.21.4";
const KEYS: &str = "/settings/content-sources";

async fn set_key(platform: &Platform, key: &str) -> Value {
    let response = platform
        .call(
            Method::PUT,
            &format!("{KEYS}/curseforge"),
            Some(json!({ "api_key": key })),
        )
        .await;
    json!({ "status": response.status.as_u16(), "body": response.json })
}

/// Serveur Minecraft Fabric en 1.21.4, déployé et arrêté.
async fn fabric(platform: &Platform, daemon: &mut FakeDaemon) -> String {
    let (server, _) = platform
        .deploy_minecraft(daemon, json!({ "loader": "fabric", "version": GAME }))
        .await;
    let id = server["id"].as_str().unwrap().to_owned();
    daemon.report(&id, "offline").await;
    platform.wait_for_state(&id, "offline").await;
    id
}

#[tokio::test]
async fn the_key_is_checked_kept_encrypted_and_never_returned() {
    let platform = Platform::start().await;

    let refused = set_key(&platform, "wrong-key").await;
    assert_eq!(refused["status"], 422, "{refused}");
    assert_eq!(refused["body"]["fields"][0]["field"], "api_key");
    assert_eq!(
        refused["body"]["fields"][0]["code"],
        "content_source_unauthorized"
    );
    assert_eq!(platform.admin.curseforge.key(), None);

    let saved = set_key(&platform, CURSEFORGE_KEY).await;
    assert_eq!(saved["status"], 204, "{saved}");
    assert_eq!(
        platform.admin.curseforge.key().as_deref(),
        Some(CURSEFORGE_KEY)
    );
    let listed = platform.call(Method::GET, KEYS, None).await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.json);
    assert_eq!(listed.json[0]["provider"], "curseforge");
    assert_eq!(listed.json[0]["configured"], true);
    assert!(!listed.json.to_string().contains(CURSEFORGE_KEY));
    let stored: Vec<u8> = sqlx::query_scalar("SELECT api_key FROM content_sources")
        .fetch_one(&platform.admin.pool)
        .await
        .unwrap();
    assert!(
        !String::from_utf8_lossy(&stored).contains(CURSEFORGE_KEY),
        "the key is stored encrypted"
    );
    let event = platform.admin.last_audit("content_source.updated").await;
    assert_eq!(event.target_id.as_deref(), Some("curseforge"));
    assert!(!event.details.to_string().contains(CURSEFORGE_KEY));

    let removed = platform
        .call(Method::DELETE, &format!("{KEYS}/curseforge"), None)
        .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    assert_eq!(platform.admin.curseforge.key(), None);
    platform.admin.last_audit("content_source.removed").await;
    let again = platform
        .call(Method::DELETE, &format!("{KEYS}/curseforge"), None)
        .await;
    assert_eq!(again.status, StatusCode::NOT_FOUND);
    assert_eq!(again.json["code"], "content_source_not_configured");
    let unknown = set_key_on(&platform, "steam", "x").await;
    assert_eq!(unknown, StatusCode::NOT_FOUND);
}

async fn set_key_on(platform: &Platform, provider: &str, key: &str) -> StatusCode {
    platform
        .call(
            Method::PUT,
            &format!("{KEYS}/{provider}"),
            Some(json!({ "api_key": key })),
        )
        .await
        .status
}

#[tokio::test]
async fn the_key_needs_its_own_permission() {
    let platform = Platform::start().await;
    let admin = &platform.admin;
    let account = admin
        .account("settings@example.com", &[Permission::SystemSettingsUpdate])
        .await;
    let token = admin.login(&account).await.access_token;

    let list = admin.call(Method::GET, KEYS, Some(&token), None).await;
    let put = admin
        .call(
            Method::PUT,
            &format!("{KEYS}/curseforge"),
            Some(&token),
            Some(json!({ "api_key": CURSEFORGE_KEY })),
        )
        .await;

    assert_eq!(list.status, StatusCode::FORBIDDEN);
    assert_eq!(put.status, StatusCode::FORBIDDEN);
    assert_eq!(admin.curseforge.key(), None);
    assert!(!Permission::SystemContentSourcesManage.agent_grantable());
    let _ = PASSWORD;
}

#[tokio::test]
async fn curseforge_waits_for_its_key_while_modrinth_works() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = fabric(&platform, &mut daemon).await;

    let slots = platform
        .call(Method::GET, &format!("/servers/{id}/content"), None)
        .await;
    let mods = slots
        .json
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["slot"] == "mods")
        .unwrap();
    assert_eq!(mods["mode"], "download");
    assert_eq!(
        mods["sources"],
        json!([
            { "provider": "modrinth", "blocked": null },
            { "provider": "curseforge", "blocked": "content_source_not_configured" },
        ])
    );
    assert_eq!(
        mods["blocked"],
        Value::Null,
        "Modrinth stays the default source"
    );
    let search = platform
        .call(
            Method::GET,
            &format!("/servers/{id}/content/mods/search?source=curseforge"),
            None,
        )
        .await;
    assert_eq!(search.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(search.json["code"], "content_source_not_configured");

    set_key(&platform, CURSEFORGE_KEY).await;
    let slots = platform
        .call(Method::GET, &format!("/servers/{id}/content"), None)
        .await;
    let mods = slots
        .json
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["slot"] == "mods")
        .unwrap();
    assert_eq!(mods["sources"][1]["blocked"], Value::Null);
}

#[tokio::test]
async fn a_curseforge_mod_installs_with_its_dependency_checked_by_sha1() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = fabric(&platform, &mut daemon).await;
    set_key(&platform, CURSEFORGE_KEY).await;
    let source = &platform.admin.curseforge;
    source.add(
        "Architectury",
        curseforge_version(2001, 200, GAME, "fabric", &[]),
    );
    source.add(
        "Waystones",
        curseforge_version(1001, 100, GAME, "fabric", &[200]),
    );
    source.add(
        "Forge Only",
        curseforge_version(3001, 300, GAME, "forge", &[]),
    );

    let search = platform
        .call(
            Method::GET,
            &format!("/servers/{id}/content/mods/search?source=curseforge"),
            None,
        )
        .await;
    let mut titles: Vec<&str> = search.json["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["title"].as_str().unwrap())
        .collect();
    titles.sort_unstable();
    assert_eq!(titles, ["Architectury", "Waystones"]);

    let path = format!("/servers/{id}/content/mods");
    let (response, request) = tokio::join!(
        platform.call(
            Method::POST,
            &path,
            Some(json!({ "version_id": "1001", "source": "curseforge" })),
        ),
        daemon.accept_content(),
    );

    assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.json);
    let files = request["payload"]["files"].as_array().unwrap();
    assert_eq!(files.len(), 2, "the mod and its required dependency");
    for file in files {
        assert_eq!(file["hash"]["algo"], "sha1");
        assert!(
            file["url"]
                .as_str()
                .unwrap()
                .starts_with("https://edge.forgecdn.net/")
        );
    }
    let items = response.json.as_array().unwrap();
    assert!(items.iter().all(|i| i["source"] == "curseforge"));
    let event = platform.admin.last_audit("server.content_installed").await;
    assert_eq!(event.details["source"], "curseforge");
    assert_eq!(event.details["project_id"], "100");

    // Une nouvelle version publiée : proposée en mise à jour.
    daemon
        .event(
            "content.done",
            json!({ "server_id": id, "operation_id": request["payload"]["operation_id"], "status": "ok" }),
        )
        .await;
    source.add(
        "Waystones",
        curseforge_version(1002, 100, GAME, "fabric", &[200]),
    );
    let mut update = Value::Null;
    for _ in 0..50 {
        let list = platform.call(Method::GET, &path, None).await;
        let waystones = list.json["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["project_id"] == "100")
            .cloned()
            .unwrap();
        if waystones["state"] == "installed" {
            update = waystones["update"].clone();
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(update["version_id"], "1002", "{update}");
}

#[tokio::test]
async fn a_mod_whose_author_forbids_distribution_is_never_downloaded() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = fabric(&platform, &mut daemon).await;
    set_key(&platform, CURSEFORGE_KEY).await;
    let source = &platform.admin.curseforge;
    source.add(
        "Closed Mod",
        curseforge_version(4001, 400, GAME, "fabric", &[]),
    );
    source.forbid("400");

    let search = platform
        .call(
            Method::GET,
            &format!("/servers/{id}/content/mods/search?source=curseforge"),
            None,
        )
        .await;
    let install = platform
        .call(
            Method::POST,
            &format!("/servers/{id}/content/mods"),
            Some(json!({ "version_id": "4001", "source": "curseforge" })),
        )
        .await;

    assert_eq!(search.json["hits"][0]["download_allowed"], false);
    assert!(search.json["hits"][0]["page_url"].is_string());
    assert_eq!(install.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(install.json["code"], "content_distribution_forbidden");
}

/// Prefab de test en mode `reference` : les mods CurseForge sont passés au jeu par leurs
/// identifiants (`MODS`), comme ARK les reçoit dans `-mods=`.
fn reference_prefab() -> Vec<u8> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../prefabs/minecraft-java");
    // Fins de ligne normalisées : un checkout Windows du sous-module peut être en CRLF.
    let manifest = std::fs::read_to_string(dir.join("prefab.yaml"))
        .unwrap()
        .replace("\r\n", "\n");
    let start = manifest.find("content:\n").unwrap();
    let end = manifest.find("# Sauvegarde à chaud").unwrap();
    let manifest = format!(
        "{}content:\n  mods:\n    kind: mods\n    service: server\n    providers:\n      - {{ provider: curseforge, game: ark-survival-ascended, class: mods }}\n    install: {{ mode: reference }}\n    fields:\n      mod_ids: {{ type: list }}\n\n{}",
        &manifest[..start],
        &manifest[end..]
    )
    .replace("id: minecraft-java", "id: reference-test")
    .replace(
        "      EULA: \"TRUE\"\n",
        "      EULA: \"TRUE\"\n      MODS: \"{{ content.mods.ids | join(\\\",\\\") }}\"\n      MOD_IDS: \"{{ content.mods.values.mod_ids | prefix(\\\"@\\\") | join(\\\",\\\") }}\"\n",
    );
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default();
    writer.start_file("prefab.yaml", options).unwrap();
    writer.write_all(manifest.as_bytes()).unwrap();
    for lang in ["en", "fr"] {
        let mut messages: Value = serde_json::from_slice(
            &std::fs::read(dir.join(format!("locales/{lang}.json"))).unwrap(),
        )
        .unwrap();
        messages["content"]["mods"]["fields"] = json!({ "mod_ids": { "label": "Mod IDs" } });
        writer
            .start_file(format!("locales/{lang}.json"), options)
            .unwrap();
        writer.write_all(messages.to_string().as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

#[tokio::test]
async fn reference_mods_reach_the_game_through_a_redeployment() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let imported = platform
        .admin
        .call_bytes(
            Method::POST,
            "/prefabs/import",
            &platform.token,
            reference_prefab(),
            true,
        )
        .await;
    assert_eq!(imported.status, StatusCode::CREATED, "{}", imported.json);
    let (created, first) = tokio::join!(
        platform.call(
            Method::POST,
            "/servers",
            Some(json!({
                "name": "Ark",
                "node_id": daemon.node.id.as_uuid(),
                "prefab_id": "reference-test",
                "memory_mb": 2048,
                "cpu_percent": 200,
                "disk_mb": 10240,
            })),
        ),
        daemon.accept(),
    );
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json);
    assert_eq!(first["payload"]["services"][0]["env"]["MODS"], "");
    let id = created.json["id"].as_str().unwrap().to_owned();
    daemon.report(&id, "offline").await;
    platform.wait_for_state(&id, "offline").await;
    set_key(&platform, CURSEFORGE_KEY).await;
    let source = &platform.admin.curseforge;
    source.add(
        "Structures Plus",
        curseforge_version(5001, 500, "", "", &[]),
    );
    source.add("Dino Storage", curseforge_version(6001, 600, "", "", &[]));
    // L'auteur refuse la distribution par des tiers : sans effet, le jeu télécharge lui-même.
    source.forbid("600");

    let slots = platform
        .call(Method::GET, &format!("/servers/{id}/content"), None)
        .await;
    assert_eq!(slots.json[0]["mode"], "reference");
    let path = format!("/servers/{id}/content/mods");
    for version in ["5001", "6001"] {
        let (response, deploy) = tokio::join!(
            platform.call(Method::POST, &path, Some(json!({ "version_id": version }))),
            daemon.accept(),
        );
        assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.json);
        assert_eq!(deploy["type"], "server.deploy");
        assert_eq!(response.json[0]["kind"], "reference");
        assert_eq!(response.json[0]["state"], "installed");
        // Le nœud rapporte l'état du serveur redéployé.
        daemon.report(&id, "offline").await;
        platform.wait_for_state(&id, "offline").await;
    }
    let list = platform.call(Method::GET, &path, None).await;
    let items = list.json["items"].as_array().unwrap();
    let dino = items.iter().find(|i| i["project_id"] == "600").unwrap();

    // Désactivé : le jeu ne le reçoit plus.
    let dino_path = format!("{path}/{}", dino["id"].as_str().unwrap());
    let (toggled, deploy) = tokio::join!(
        platform.call(Method::PATCH, &dino_path, Some(json!({ "enabled": false }))),
        daemon.accept(),
    );
    assert_eq!(toggled.status, StatusCode::OK, "{}", toggled.json);
    assert_eq!(deploy["payload"]["services"][0]["env"]["MODS"], "500");
    daemon.report(&id, "offline").await;
    platform.wait_for_state(&id, "offline").await;

    let structures = items.iter().find(|i| i["project_id"] == "500").unwrap();
    let structures_path = format!("{path}/{}", structures["id"].as_str().unwrap());
    let (removed, deploy) = tokio::join!(
        platform.call(Method::DELETE, &structures_path, None),
        daemon.accept(),
    );
    assert_eq!(removed.status, StatusCode::OK, "{}", removed.json);
    assert_eq!(deploy["payload"]["services"][0]["env"]["MODS"], "");
    let event = platform.admin.last_audit("server.content_installed").await;
    assert_eq!(event.details["mode"], "reference");
}

fn category(id: &str, name: &str) -> kuro_api::content::Category {
    kuro_api::content::Category {
        id: id.to_owned(),
        name: name.to_owned(),
        icon_svg: None,
        icon_url: None,
    }
}

#[tokio::test]
async fn the_search_sorts_and_filters_by_categories_of_the_chosen_source() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = fabric(&platform, &mut daemon).await;
    set_key(&platform, CURSEFORGE_KEY).await;
    *platform.admin.curseforge.categories.lock().unwrap() = vec![
        category("421", "Map and Information"),
        category("424", "Storage"),
    ];
    *platform.admin.content.categories.lock().unwrap() = vec![category("magic", "Magic")];

    let curseforge = platform
        .call(
            Method::GET,
            &format!("/servers/{id}/content/mods/categories?source=curseforge"),
            None,
        )
        .await;
    let modrinth = platform
        .call(
            Method::GET,
            &format!("/servers/{id}/content/mods/categories"),
            None,
        )
        .await;
    assert_eq!(curseforge.status, StatusCode::OK, "{}", curseforge.json);
    assert_eq!(curseforge.json[0]["id"], "421");
    assert_eq!(curseforge.json[1]["name"], "Storage");
    assert_eq!(
        modrinth.json,
        json!([{ "id": "magic", "name": "Magic", "icon_svg": null, "icon_url": null }])
    );

    let search = platform
        .call(
            Method::GET,
            &format!(
                "/servers/{id}/content/mods/search?source=curseforge&q=map&sort=downloads&categories=421,424"
            ),
            None,
        )
        .await;
    assert_eq!(search.status, StatusCode::OK, "{}", search.json);
    let received = platform
        .admin
        .curseforge
        .last_search
        .lock()
        .unwrap()
        .clone()
        .unwrap();
    assert_eq!(received.sort, kuro_api::content::SearchSort::Downloads);
    assert_eq!(received.categories, ["421", "424"]);
    assert_eq!(received.text, "map");
    // Sans filtre : pertinence, aucune catégorie, source par défaut.
    platform
        .call(
            Method::GET,
            &format!("/servers/{id}/content/mods/search"),
            None,
        )
        .await;
    let received = platform
        .admin
        .content
        .last_search
        .lock()
        .unwrap()
        .clone()
        .unwrap();
    assert_eq!(received.sort, kuro_api::content::SearchSort::Relevance);
    assert!(received.categories.is_empty());
}

#[tokio::test]
async fn invalid_search_filters_are_refused() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = fabric(&platform, &mut daemon).await;
    let eleven: Vec<String> = (0..11).map(|n| n.to_string()).collect();

    for query in [
        "sort=popular".to_owned(),
        format!("categories={}", eleven.join(",")),
        "categories=a%2Fb".to_owned(),
    ] {
        let response = platform
            .call(
                Method::GET,
                &format!("/servers/{id}/content/mods/search?{query}"),
                None,
            )
            .await;
        assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY, "{query}");
        assert_eq!(response.json["code"], "invalid_parameter", "{query}");
    }
    assert!(platform.admin.content.last_search.lock().unwrap().is_none());
    let without_key = platform
        .call(
            Method::GET,
            &format!("/servers/{id}/content/mods/categories?source=curseforge"),
            None,
        )
        .await;
    assert_eq!(without_key.json["code"], "content_source_not_configured");
}

#[tokio::test]
async fn uploads_follow_the_prefab() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let (created, _) = tokio::join!(
        platform.call(
            Method::POST,
            "/servers",
            Some(json!({
                "name": "Zomboid",
                "node_id": daemon.node.id.as_uuid(),
                "prefab_id": "project-zomboid",
                "memory_mb": 4096,
                "cpu_percent": 200,
                "disk_mb": 10240,
            })),
        ),
        daemon.accept(),
    );
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json);
    let id = created.json["id"].as_str().unwrap().to_owned();
    daemon.report(&id, "offline").await;
    platform.wait_for_state(&id, "offline").await;

    let slots = platform
        .call(Method::GET, &format!("/servers/{id}/content"), None)
        .await;
    let mods = &slots.json[0];
    assert_eq!(mods["mode"], "reference");
    assert_eq!(mods["upload"], false);

    // Pas d'envoi de fichier sans `provider: upload` : le fichier n'atteint jamais le nœud.
    let upload = platform
        .admin
        .call_bytes(
            Method::POST,
            &format!("/servers/{id}/content/mods/upload?file_name=mine.zip"),
            &platform.token,
            b"PK".to_vec(),
            true,
        )
        .await;
    assert_eq!(
        upload.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        upload.json
    );
    assert_eq!(upload.json["code"], "upload_not_accepted");
}

/// Serveur du prefab de test en mode `reference`, arrêté, CurseForge configuré.
async fn reference_server(platform: &Platform, daemon: &mut FakeDaemon) -> String {
    let imported = platform
        .admin
        .call_bytes(
            Method::POST,
            "/prefabs/import",
            &platform.token,
            reference_prefab(),
            true,
        )
        .await;
    assert_eq!(imported.status, StatusCode::CREATED, "{}", imported.json);
    let (created, _) = tokio::join!(
        platform.call(
            Method::POST,
            "/servers",
            Some(json!({
                "name": "Ark",
                "node_id": daemon.node.id.as_uuid(),
                "prefab_id": "reference-test",
                "memory_mb": 2048,
                "cpu_percent": 200,
                "disk_mb": 10240,
            })),
        ),
        daemon.accept(),
    );
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json);
    let id = created.json["id"].as_str().unwrap().to_owned();
    daemon.report(&id, "offline").await;
    platform.wait_for_state(&id, "offline").await;
    set_key(platform, CURSEFORGE_KEY).await;
    id
}

#[tokio::test]
async fn values_entered_for_a_mod_reach_the_game_configuration() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = reference_server(&platform, &mut daemon).await;
    platform
        .admin
        .curseforge
        .add("Hydrocraft", curseforge_version(5001, 500, "", "", &[]));
    let path = format!("/servers/{id}/content/mods");

    let slots = platform
        .call(Method::GET, &format!("/servers/{id}/content"), None)
        .await;
    assert_eq!(
        slots.json[0]["fields"],
        json!([{ "key": "mod_ids", "type": "list", "required": false, "prefill": false }])
    );

    // Refusées avant tout redéploiement : caractère de contrôle (il casserait le fichier de
    // configuration), champ inconnu, type invalide.
    for values in [
        json!({ "mod_ids": ["Hydrocraft", "bad\nline"] }),
        json!({ "unknown": "x" }),
        json!({ "mod_ids": 42 }),
    ] {
        let refused = platform
            .call(
                Method::POST,
                &path,
                Some(json!({ "version_id": "5001", "values": values })),
            )
            .await;
        assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY, "{values}");
        assert!(
            refused.json["fields"][0]["field"]
                .as_str()
                .unwrap()
                .starts_with("values."),
            "{}",
            refused.json
        );
    }

    let (installed, deploy) = tokio::join!(
        platform.call(
            Method::POST,
            &path,
            Some(json!({
                "version_id": "5001",
                "values": { "mod_ids": ["Hydrocraft", " BetterSorting ", ""] },
            })),
        ),
        daemon.accept(),
    );
    assert_eq!(installed.status, StatusCode::ACCEPTED, "{}", installed.json);
    assert_eq!(
        installed.json[0]["values"],
        json!({ "mod_ids": ["Hydrocraft", "BetterSorting"] })
    );
    assert_eq!(
        deploy["payload"]["services"][0]["env"]["MOD_IDS"],
        "@Hydrocraft,@BetterSorting"
    );
    daemon.report(&id, "offline").await;
    platform.wait_for_state(&id, "offline").await;

    let content_id = installed.json[0]["id"].as_str().unwrap().to_owned();
    let values_path = format!("{path}/{content_id}/values");
    let (changed, deploy) = tokio::join!(
        platform.call(
            Method::PUT,
            &values_path,
            Some(json!({ "values": { "mod_ids": ["Hydrocraft"] } })),
        ),
        daemon.accept(),
    );
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.json);
    assert_eq!(
        deploy["payload"]["services"][0]["env"]["MOD_IDS"],
        "@Hydrocraft"
    );
    let event = platform
        .admin
        .last_audit("server.content_values_updated")
        .await;
    assert_eq!(event.details["fields"], json!(["mod_ids"]));
}
