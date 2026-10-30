//! Contenu des serveurs (ADR-0020) : emplacements, recherche compatible, installation avec les
//! dépendances, fin rapportée par le nœud, mise à jour, activation, suppression, refus.

use axum::http::{Method, StatusCode};
use kuro_api::iam::domain::Permission;
use serde_json::{Value, json};

use crate::support::content::{sha, version};
use crate::support::platform::{FakeDaemon, Platform};

const GAME: &str = "1.21.4";

/// Serveur Minecraft Fabric en 1.21.4, déployé et arrêté.
async fn server(platform: &Platform, daemon: &mut FakeDaemon, settings: Value) -> String {
    let (server, _) = platform.deploy_minecraft(daemon, settings).await;
    let id = server["id"].as_str().unwrap().to_owned();
    daemon.report(&id, "offline").await;
    platform.wait_for_state(&id, "offline").await;
    id
}

async fn fabric(platform: &Platform, daemon: &mut FakeDaemon) -> String {
    server(
        platform,
        daemon,
        json!({ "loader": "fabric", "version": GAME }),
    )
    .await
}

/// Catalogue : Sodium exige Fabric API ; un mod d'une autre version du jeu.
fn catalog(platform: &Platform) {
    let fake = &platform.admin.content;
    fake.add(
        "Fabric API",
        version("fapi-1", "fabric-api", GAME, "fabric", &[]),
    );
    fake.add(
        "Sodium",
        version("sodium-1", "sodium", GAME, "fabric", &["fabric-api"]),
    );
    fake.add(
        "Old Mod",
        version("old-1", "old-mod", "1.20.1", "fabric", &[]),
    );
}

async fn items(platform: &Platform, id: &str) -> Vec<Value> {
    platform
        .call(Method::GET, &format!("/servers/{id}/content/mods"), None)
        .await
        .json["items"]
        .as_array()
        .unwrap()
        .clone()
}

/// Attend que tous les contenus quittent `installing`.
async fn settled(platform: &Platform, id: &str) -> Vec<Value> {
    for _ in 0..100 {
        let list = items(platform, id).await;
        if list.iter().all(|i| i["state"] != "installing") {
            return list;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("content stayed installing");
}

fn named<'a>(list: &'a [Value], name: &str) -> &'a Value {
    list.iter()
        .find(|i| i["name"] == name)
        .unwrap_or_else(|| panic!("no {name}"))
}

/// Installe une version ; le faux nœud accepte et termine l'opération.
async fn install(
    platform: &Platform,
    daemon: &mut FakeDaemon,
    id: &str,
    version_id: &str,
) -> Value {
    let path = format!("/servers/{id}/content/mods");
    let (response, request) = tokio::join!(
        platform.call(
            Method::POST,
            &path,
            Some(json!({ "version_id": version_id }))
        ),
        daemon.accept_content(),
    );
    assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.json);
    daemon
        .event(
            "content.done",
            json!({ "server_id": id, "operation_id": request["payload"]["operation_id"], "status": "ok" }),
        )
        .await;
    request
}

#[tokio::test]
async fn a_slot_needs_a_mod_loader_and_a_pinned_game_version() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let vanilla = server(&platform, &mut daemon, json!({})).await;
    let latest = server(&platform, &mut daemon, json!({ "loader": "fabric" })).await;
    let pinned = fabric(&platform, &mut daemon).await;

    let slot = |list: Value| {
        list.as_array()
            .unwrap()
            .iter()
            .find(|s| s["slot"] == "mods")
            .unwrap()
            .clone()
    };
    let vanilla = slot(
        platform
            .call(Method::GET, &format!("/servers/{vanilla}/content"), None)
            .await
            .json,
    );
    let latest = slot(
        platform
            .call(Method::GET, &format!("/servers/{latest}/content"), None)
            .await
            .json,
    );
    let pinned = slot(
        platform
            .call(Method::GET, &format!("/servers/{pinned}/content"), None)
            .await
            .json,
    );

    assert_eq!(vanilla["blocked"], "content_not_supported");
    assert_eq!(latest["blocked"], "game_version_not_pinned");
    assert_eq!(latest["latest_game_version"], GAME);
    assert!(pinned["blocked"].is_null());
    assert_eq!(pinned["loader"], "fabric");
    assert_eq!(pinned["game_version"], GAME);
}

#[tokio::test]
async fn search_and_versions_are_limited_to_what_the_server_can_load() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = fabric(&platform, &mut daemon).await;
    catalog(&platform);

    let all = platform
        .call(
            Method::GET,
            &format!("/servers/{id}/content/mods/search"),
            None,
        )
        .await;
    let old = platform
        .call(
            Method::GET,
            &format!("/servers/{id}/content/mods/projects/old-mod/versions"),
            None,
        )
        .await;
    let sodium = platform
        .call(
            Method::GET,
            &format!("/servers/{id}/content/mods/projects/sodium/versions"),
            None,
        )
        .await;

    assert_eq!(all.status, StatusCode::OK, "{}", all.json);
    let titles: Vec<&str> = all.json["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["Fabric API", "Sodium"]);
    assert_eq!(old.json, json!([]));
    assert_eq!(sodium.json[0]["required_projects"], json!(["fabric-api"]));
}

#[tokio::test]
async fn a_quilt_server_finds_quilt_and_fabric_mods() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = server(
        &platform,
        &mut daemon,
        json!({ "loader": "quilt", "version": GAME }),
    )
    .await;
    catalog(&platform);
    let fake = &platform.admin.content;
    fake.add(
        "Quilted Fabric API",
        version("qfapi-1", "qfapi", GAME, "quilt", &[]),
    );
    fake.add(
        "Forge Mod",
        version("forge-1", "forge-mod", GAME, "forge", &[]),
    );

    let all = platform
        .call(
            Method::GET,
            &format!("/servers/{id}/content/mods/search"),
            None,
        )
        .await;

    assert_eq!(all.status, StatusCode::OK, "{}", all.json);
    let mut titles: Vec<&str> = all.json["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["title"].as_str().unwrap())
        .collect();
    titles.sort_unstable();
    assert_eq!(titles, ["Fabric API", "Quilted Fabric API", "Sodium"]);
}

#[tokio::test]
async fn a_mod_comes_with_its_required_dependencies() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = fabric(&platform, &mut daemon).await;
    catalog(&platform);

    let request = install(&platform, &mut daemon, &id, "sodium-1").await;

    assert_eq!(request["type"], "content.install");
    let payload = &request["payload"];
    assert_eq!(payload["service"], "server");
    assert_eq!(payload["dir"], "/data/mods");
    let files: Vec<&str> = payload["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["file_name"].as_str().unwrap())
        .collect();
    assert_eq!(files, ["sodium-sodium-1.jar", "fabric-api-fapi-1.jar"]);
    assert_eq!(
        payload["files"][0]["hash"],
        json!({ "algo": "sha512", "value": sha("sodium-1") })
    );
    assert!(
        payload["files"][0]["url"]
            .as_str()
            .unwrap()
            .starts_with("https://cdn.modrinth.com/")
    );

    let list = settled(&platform, &id).await;
    let sodium = named(&list, "Sodium");
    let fapi = named(&list, "Fabric API");
    assert_eq!(sodium["state"], "installed");
    assert_eq!(fapi["required_by"], json!([sodium["id"]]));
    assert_eq!(sodium["orphans"], json!([fapi["id"]]));
    let event = platform.admin.last_audit("server.content_installed").await;
    assert_eq!(event.details["project_id"], "sodium");
    assert_eq!(event.details["dependencies"], json!(["fabric-api"]));
}

#[tokio::test]
async fn an_installed_dependency_is_linked_not_downloaded_again() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = fabric(&platform, &mut daemon).await;
    catalog(&platform);
    install(&platform, &mut daemon, &id, "fapi-1").await;
    settled(&platform, &id).await;

    let request = install(&platform, &mut daemon, &id, "sodium-1").await;

    assert_eq!(request["payload"]["files"].as_array().unwrap().len(), 1);
    let list = settled(&platform, &id).await;
    assert_eq!(
        named(&list, "Fabric API")["required_by"],
        json!([named(&list, "Sodium")["id"]])
    );
    let again = platform
        .call(
            Method::POST,
            &format!("/servers/{id}/content/mods"),
            Some(json!({ "version_id": "fapi-1" })),
        )
        .await;
    assert_eq!(again.json["code"], "content_already_installed");
}

#[tokio::test]
async fn a_node_failure_is_kept_and_a_retry_replaces_it() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = fabric(&platform, &mut daemon).await;
    catalog(&platform);

    let path = format!("/servers/{id}/content/mods");
    let (response, request) = tokio::join!(
        platform.call(Method::POST, &path, Some(json!({ "version_id": "fapi-1" }))),
        daemon.accept_content(),
    );
    assert_eq!(response.status, StatusCode::ACCEPTED);
    daemon
        .event(
            "content.done",
            json!({ "server_id": id, "operation_id": request["payload"]["operation_id"], "status": "failed", "reason": "checksum mismatch" }),
        )
        .await;
    let list = settled(&platform, &id).await;
    assert_eq!(list[0]["state"], "failed");
    assert_eq!(list[0]["failure"], "checksum mismatch");

    install(&platform, &mut daemon, &id, "fapi-1").await;
    let list = settled(&platform, &id).await;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["state"], "installed");
}

#[tokio::test]
async fn a_refusal_of_the_node_is_audited_as_a_failure() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = fabric(&platform, &mut daemon).await;
    catalog(&platform);

    let path = format!("/servers/{id}/content/mods");
    let (response, ()) = tokio::join!(
        platform.call(Method::POST, &path, Some(json!({ "version_id": "fapi-1" }))),
        async {
            let request = daemon.next_content_request().await;
            daemon
                .reply(
                    &request,
                    json!({ "status": "error", "code": "disk_quota_exceeded", "message": "full" }),
                )
                .await;
        },
    );

    assert_eq!(response.status, StatusCode::CONFLICT);
    assert_eq!(response.json["code"], "disk_quota_exceeded");
    assert_eq!(items(&platform, &id).await[0]["state"], "failed");
    let failure = platform
        .admin
        .audit()
        .await
        .into_iter()
        .rev()
        .find(|row| row.action == "server.content_installed")
        .unwrap();
    assert_eq!(failure.outcome, "failure");
}

#[tokio::test]
async fn a_required_dependency_cannot_go_alone_but_leaves_with_its_mod() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = fabric(&platform, &mut daemon).await;
    catalog(&platform);
    install(&platform, &mut daemon, &id, "sodium-1").await;
    let list = settled(&platform, &id).await;
    let (sodium, fapi) = (
        named(&list, "Sodium")["id"].clone(),
        named(&list, "Fabric API")["id"].clone(),
    );

    let alone = platform
        .call(
            Method::DELETE,
            &format!("/servers/{id}/content/mods/{}", fapi.as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(alone.status, StatusCode::CONFLICT);
    assert_eq!(alone.json["code"], "content_required");

    let path = format!(
        "/servers/{id}/content/mods/{}?with_orphans=true",
        sodium.as_str().unwrap()
    );
    let (response, request) = tokio::join!(
        platform.call(Method::DELETE, &path, None),
        daemon.accept_content(),
    );
    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(request["type"], "content.remove");
    let mut removed: Vec<&str> = request["payload"]["file_names"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();
    removed.sort_unstable();
    assert_eq!(removed, ["fabric-api-fapi-1.jar", "sodium-sodium-1.jar"]);
    assert_eq!(items(&platform, &id).await, Vec::<Value>::new());
    platform.admin.last_audit("server.content_removed").await;
}

#[tokio::test]
async fn an_update_replaces_the_old_version_once_installed() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = fabric(&platform, &mut daemon).await;
    catalog(&platform);
    install(&platform, &mut daemon, &id, "sodium-1").await;
    settled(&platform, &id).await;
    platform.admin.content.add(
        "Sodium",
        version("sodium-2", "sodium", GAME, "fabric", &["fabric-api"]),
    );

    let list = items(&platform, &id).await;
    let sodium = named(&list, "Sodium").clone();
    assert_eq!(sodium["update"]["version_id"], "sodium-2");
    assert!(named(&list, "Fabric API")["update"].is_null());

    let path = format!(
        "/servers/{id}/content/mods/{}/update",
        sodium["id"].as_str().unwrap()
    );
    let (response, request) = tokio::join!(
        platform.call(Method::POST, &path, None),
        daemon.accept_content()
    );
    assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.json);
    assert_eq!(
        request["payload"]["files"][0]["file_name"],
        "sodium-sodium-2.jar"
    );
    assert_eq!(request["payload"]["files"].as_array().unwrap().len(), 1);
    daemon
        .event(
            "content.done",
            json!({ "server_id": id, "operation_id": request["payload"]["operation_id"], "status": "ok" }),
        )
        .await;
    // L'ancien fichier est retiré du nœud.
    let removal = daemon.accept_content().await;
    assert_eq!(removal["type"], "content.remove");
    assert_eq!(
        removal["payload"]["file_names"],
        json!(["sodium-sodium-1.jar"])
    );

    let list = settled(&platform, &id).await;
    assert_eq!(list.len(), 2);
    let sodium = named(&list, "Sodium");
    assert_eq!(sodium["version_id"], "sodium-2");
    assert_eq!(
        named(&list, "Fabric API")["required_by"],
        json!([sodium["id"]])
    );
    let event = platform.admin.last_audit("server.content_updated").await;
    assert_eq!(event.details["from"], "sodium-1");
    assert_eq!(event.details["to"], "sodium-2");
}

#[tokio::test]
async fn a_mod_is_disabled_without_being_removed() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = fabric(&platform, &mut daemon).await;
    catalog(&platform);
    install(&platform, &mut daemon, &id, "fapi-1").await;
    let content_id = settled(&platform, &id).await[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let path = format!("/servers/{id}/content/mods/{content_id}");
    let (response, request) = tokio::join!(
        platform.call(Method::PATCH, &path, Some(json!({ "enabled": false }))),
        daemon.accept_content(),
    );

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(response.json["enabled"], false);
    assert_eq!(request["type"], "content.toggle");
    assert_eq!(request["payload"]["file_name"], "fabric-api-fapi-1.jar");
    assert_eq!(request["payload"]["enabled"], false);
    platform.admin.last_audit("server.content_toggled").await;
}

#[tokio::test]
async fn incompatible_or_foreign_files_never_reach_the_node() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = fabric(&platform, &mut daemon).await;
    catalog(&platform);
    let mut foreign = version("evil-1", "evil", GAME, "fabric", &[]);
    foreign.files[0].url = "https://example.com/evil.jar".to_owned();
    platform.admin.content.add("Evil", foreign);
    let path = format!("/servers/{id}/content/mods");

    let old = platform
        .call(Method::POST, &path, Some(json!({ "version_id": "old-1" })))
        .await;
    let evil = platform
        .call(Method::POST, &path, Some(json!({ "version_id": "evil-1" })))
        .await;
    let missing = platform
        .call(Method::POST, &path, Some(json!({ "version_id": "nope" })))
        .await;

    assert_eq!(old.json["code"], "content_incompatible");
    assert_eq!(evil.json["code"], "content_incompatible");
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(items(&platform, &id).await, Vec::<Value>::new());
}

#[tokio::test]
async fn an_unreachable_provider_fails_the_search_but_not_the_list() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = fabric(&platform, &mut daemon).await;
    catalog(&platform);
    install(&platform, &mut daemon, &id, "fapi-1").await;
    settled(&platform, &id).await;
    *platform.admin.content.unavailable.lock().unwrap() = true;

    let search = platform
        .call(
            Method::GET,
            &format!("/servers/{id}/content/mods/search?q=x"),
            None,
        )
        .await;
    let list = platform
        .call(Method::GET, &format!("/servers/{id}/content/mods"), None)
        .await;

    assert_eq!(search.status, StatusCode::BAD_GATEWAY);
    assert_eq!(search.json["code"], "provider_unavailable");
    assert_eq!(list.status, StatusCode::OK);
    assert_eq!(list.json["items"][0]["name"], "Fabric API");
}

#[tokio::test]
async fn reading_and_managing_content_need_their_permissions() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = fabric(&platform, &mut daemon).await;
    catalog(&platform);
    let reader = platform
        .admin
        .account(
            "reader@example.com",
            &[Permission::ServersRead, Permission::ServersContentRead],
        )
        .await;
    let token = platform.admin.login(&reader).await.access_token;
    let path = format!("/servers/{id}/content/mods");

    let list = platform
        .admin
        .call(Method::GET, &path, Some(&token), None)
        .await;
    let install = platform
        .admin
        .call(
            Method::POST,
            &path,
            Some(&token),
            Some(json!({ "version_id": "fapi-1" })),
        )
        .await;

    assert_eq!(list.status, StatusCode::OK);
    assert_eq!(install.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_report_from_another_node_is_ignored() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let mut other = platform.online_node("node-2", "25565-25570").await;
    let id = fabric(&platform, &mut daemon).await;
    catalog(&platform);
    let path = format!("/servers/{id}/content/mods");
    let (_, request) = tokio::join!(
        platform.call(Method::POST, &path, Some(json!({ "version_id": "fapi-1" }))),
        daemon.accept_content(),
    );

    other
        .event(
            "content.done",
            json!({ "server_id": id, "operation_id": request["payload"]["operation_id"], "status": "ok" }),
        )
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    assert_eq!(items(&platform, &id).await[0]["state"], "installing");
}
