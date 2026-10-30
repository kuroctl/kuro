//! Add-ons Bedrock (ADR-0030, mode `extract`) : le nœud extrait l'archive en paquets rangés
//! d'après leur `manifest.json` ; l'API garde leurs descriptions, les inscrit dans le monde
//! (`world_*_packs.json`), les retire de l'inscription quand ils sont désactivés et fait
//! supprimer leurs dossiers avec eux.

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

use crate::support::content::{CURSEFORGE_KEY, curseforge_version};
use crate::support::platform::{FakeDaemon, Platform};

const BEHAVIOR: &str = "/data/worlds/Bedrock level/world_behavior_packs.json";
const RESOURCES: &str = "/data/worlds/Bedrock level/world_resource_packs.json";

/// Serveur Bedrock déployé et arrêté, avec la clé CurseForge et un add-on publié.
async fn bedrock(platform: &Platform, daemon: &mut FakeDaemon) -> String {
    let (created, _) = tokio::join!(
        platform.call(
            Method::POST,
            "/servers",
            Some(json!({
                "name": "Bedrock",
                "node_id": daemon.node.id.as_uuid(),
                "prefab_id": "minecraft-bedrock",
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
    let key = platform
        .call(
            Method::PUT,
            "/settings/content-sources/curseforge",
            Some(json!({ "api_key": CURSEFORGE_KEY })),
        )
        .await;
    assert_eq!(key.status, StatusCode::NO_CONTENT, "{}", key.json);
    let mut addon = curseforge_version(5001, 500, "", "", &[]);
    addon.files[0].file_name = "more-mobs.mcaddon".to_owned();
    platform.admin.curseforge.add("More Mobs", addon);
    id
}

/// Fichier d'inscription d'un message `server.deploy`, lu en JSON.
fn registered(deploy: &Value, path: &str) -> Value {
    let file = deploy["payload"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["path"] == path)
        .unwrap_or_else(|| panic!("no {path} in {deploy}"));
    assert_eq!(file["mode"], "template");
    serde_json::from_str(file["content"].as_str().unwrap()).unwrap()
}

fn package(name: &str, uuid: &str, kind: &str) -> Value {
    json!({
        "file_name": "more-mobs.mcaddon",
        "dir": if kind == "data" { "/data/behavior_packs" } else { "/data/resource_packs" },
        "name": name,
        "description": {
            "header": { "name": "More Mobs", "uuid": uuid, "version": [1, 2, 0] },
            "modules": [{ "type": kind }],
        },
    })
}

#[tokio::test]
async fn an_addon_is_extracted_registered_toggled_and_removed() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "19132-19135").await;
    let id = bedrock(&platform, &mut daemon).await;
    let path = format!("/servers/{id}/content/addons");

    // Le nœud reçoit l'archive à extraire et les règles de rangement du prefab.
    let (response, request) = tokio::join!(
        platform.call(
            Method::POST,
            &path,
            Some(json!({ "version_id": "5001", "source": "curseforge" })),
        ),
        daemon.accept_content(),
    );
    assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.json);
    let payload = &request["payload"];
    assert_eq!(payload["files"][0]["file_name"], "more-mobs.mcaddon");
    assert_eq!(payload["extract"]["describe"], "manifest.json");
    assert_eq!(payload["extract"]["routes"].as_array().unwrap().len(), 3);
    assert_eq!(payload["dir"], "/data/behavior_packs");

    // Ses paquets rapportés : enregistrés, puis inscrits dans le monde au redéploiement.
    daemon
        .event(
            "content.done",
            json!({
                "server_id": id,
                "operation_id": payload["operation_id"],
                "status": "ok",
                "packages": [package("more-mobs-1", "b-1", "data"), package("more-mobs-2", "r-1", "resources")],
            }),
        )
        .await;
    let deploy = daemon.accept().await;
    assert_eq!(deploy["type"], "server.deploy");
    assert_eq!(
        registered(&deploy, BEHAVIOR),
        json!([{ "pack_id": "b-1", "version": [1, 2, 0] }])
    );
    assert_eq!(
        registered(&deploy, RESOURCES),
        json!([{ "pack_id": "r-1", "version": [1, 2, 0] }])
    );
    daemon.report(&id, "offline").await;
    platform.wait_for_state(&id, "offline").await;

    let list = platform.call(Method::GET, &path, None).await;
    let item = &list.json["items"][0];
    assert_eq!(item["state"], "installed");
    assert_eq!(
        item["packages"],
        json!([
            { "dir": "/data/behavior_packs", "name": "more-mobs-1", "title": "More Mobs", "version": "1.2.0" },
            { "dir": "/data/resource_packs", "name": "more-mobs-2", "title": "More Mobs", "version": "1.2.0" },
        ])
    );
    let item_path = format!("{path}/{}", item["id"].as_str().unwrap());

    // Désactivé : retiré de l'inscription, ses dossiers restent.
    let (toggled, deploy) = tokio::join!(
        platform.call(Method::PATCH, &item_path, Some(json!({ "enabled": false }))),
        daemon.accept(),
    );
    assert_eq!(toggled.status, StatusCode::OK, "{}", toggled.json);
    assert_eq!(registered(&deploy, BEHAVIOR), json!([]));
    assert_eq!(registered(&deploy, RESOURCES), json!([]));
    daemon.report(&id, "offline").await;
    platform.wait_for_state(&id, "offline").await;

    // Supprimé : le nœud efface les dossiers de ses paquets, pas l'archive déjà partie.
    let (removed, (remove, deploy)) =
        tokio::join!(platform.call(Method::DELETE, &item_path, None), async {
            let remove = daemon.accept_content().await;
            (remove, daemon.accept().await)
        },);
    assert_eq!(removed.status, StatusCode::OK, "{}", removed.json);
    assert_eq!(remove["type"], "content.remove");
    assert_eq!(
        remove["payload"]["dirs"],
        json!([
            "/data/behavior_packs/more-mobs-1",
            "/data/resource_packs/more-mobs-2"
        ])
    );
    assert_eq!(remove["payload"].get("file_names"), Some(&json!([])));
    assert_eq!(deploy["type"], "server.deploy");
    let event = platform.admin.last_audit("server.content_removed").await;
    assert_eq!(event.details["slot"], "addons");
}

#[tokio::test]
async fn an_extracted_slot_refuses_uploads_for_now() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "19132-19135").await;
    let id = bedrock(&platform, &mut daemon).await;

    let slots = platform
        .call(Method::GET, &format!("/servers/{id}/content"), None)
        .await;
    let addons = slots
        .json
        .as_array()
        .unwrap()
        .iter()
        .find(|slot| slot["slot"] == "addons")
        .unwrap();
    assert_eq!(addons["upload"], false);
    assert_eq!(addons["blocked"], Value::Null, "{addons}");
}

#[tokio::test]
async fn a_running_server_keeps_running_and_says_what_waits_for_its_restart() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "19132-19135").await;
    let id = bedrock(&platform, &mut daemon).await;
    daemon.report(&id, "running").await;
    platform.wait_for_state(&id, "running").await;
    let path = format!("/servers/{id}/content/addons");

    let (response, request) = tokio::join!(
        platform.call(
            Method::POST,
            &path,
            Some(json!({ "version_id": "5001", "source": "curseforge" })),
        ),
        daemon.accept_content(),
    );
    assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.json);
    daemon
        .event(
            "content.done",
            json!({
                "server_id": id,
                "operation_id": request["payload"]["operation_id"],
                "status": "ok",
                "packages": [package("more-mobs-1", "b-1", "data")],
            }),
        )
        .await;

    // La configuration part sans redémarrer le serveur.
    let deploy = daemon.accept().await;
    assert_eq!(deploy["type"], "server.deploy");
    assert_eq!(deploy["payload"]["defer"], true);
    assert_eq!(
        registered(&deploy, BEHAVIOR),
        json!([{ "pack_id": "b-1", "version": [1, 2, 0] }])
    );
    let server = platform
        .call(Method::GET, &format!("/servers/{id}"), None)
        .await;
    assert_eq!(server.json["state"], "running");
    assert_eq!(server.json["restart_required"], true);
    assert_eq!(
        server.json["pending_changes"],
        json!({ "contents": 1, "removals": 0 })
    );
    let list = platform.call(Method::GET, &path, None).await;
    assert_eq!(list.json["items"][0]["pending"], true);

    // Le serveur redémarre : tout est appliqué.
    daemon.report(&id, "starting").await;
    platform.wait_for_state(&id, "starting").await;
    let server = platform
        .call(Method::GET, &format!("/servers/{id}"), None)
        .await;
    assert_eq!(server.json["restart_required"], false);
    assert_eq!(
        server.json["pending_changes"],
        json!({ "contents": 0, "removals": 0 })
    );
    let list = platform.call(Method::GET, &path, None).await;
    assert_eq!(list.json["items"][0]["pending"], false);
}
