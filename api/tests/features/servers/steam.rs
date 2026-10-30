//! Steam Workshop (ADR-0030) sur le prefab officiel Project Zomboid : clé de l'API Web de Steam,
//! objets ajoutés avec leurs objets requis, identifiants de mods proposés d'après la description,
//! et `WorkshopItems=` / `Mods=` écrits dans la configuration du serveur.

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

use crate::support::content::{STEAM_KEY, version};
use crate::support::platform::{FakeDaemon, Platform};

/// Serveur Project Zomboid déployé et arrêté.
async fn zomboid(platform: &Platform, daemon: &mut FakeDaemon) -> String {
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
    id
}

/// Clés gérées de `kuro.ini` dans un message `server.deploy`.
fn ini_keys(deploy: &Value) -> Value {
    deploy["payload"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["path"].as_str().unwrap().ends_with("/kuro.ini"))
        .map(|file| file["keys"].clone())
        .unwrap()
}

/// Objet Workshop simulé : sa seule « version » porte son identifiant.
fn item(platform: &Platform, id: &str, title: &str, requires: &[&str], description: &str) {
    let steam = &platform.admin.steam;
    let mut v = version(id, id, "", "", requires);
    v.files.clear();
    for dependency in &mut v.dependencies {
        dependency.version_id = dependency.project_id.clone();
    }
    steam.add(title, v);
    steam
        .descriptions
        .lock()
        .unwrap()
        .insert(id.to_owned(), description.to_owned());
}

#[tokio::test]
async fn the_workshop_waits_for_its_steam_key() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = zomboid(&platform, &mut daemon).await;

    let slots = platform
        .call(Method::GET, &format!("/servers/{id}/content"), None)
        .await;
    assert_eq!(slots.json[0]["blocked"], "content_source_not_configured");
    assert_eq!(
        slots.json[0]["fields"],
        json!([{ "key": "mod_ids", "type": "list", "required": true, "prefill": true }])
    );
    assert_eq!(slots.json[0]["sources"][0]["provider"], "steam-workshop");

    let listed = platform
        .call(Method::GET, "/settings/content-sources", None)
        .await;
    let providers: Vec<&str> = listed
        .json
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["provider"].as_str().unwrap())
        .collect();
    assert_eq!(providers, ["curseforge", "steam-workshop", "steamgriddb"]);

    let refused = platform
        .call(
            Method::PUT,
            "/settings/content-sources/steam-workshop",
            Some(json!({ "api_key": "wrong" })),
        )
        .await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    let saved = platform
        .call(
            Method::PUT,
            "/settings/content-sources/steam-workshop",
            Some(json!({ "api_key": STEAM_KEY })),
        )
        .await;
    assert_eq!(saved.status, StatusCode::NO_CONTENT, "{}", saved.json);
    assert_eq!(platform.admin.steam.key().as_deref(), Some(STEAM_KEY));

    let slots = platform
        .call(Method::GET, &format!("/servers/{id}/content"), None)
        .await;
    assert_eq!(slots.json[0]["blocked"], Value::Null);
}

#[tokio::test]
async fn a_workshop_item_comes_with_its_required_items_and_their_mod_ids() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = zomboid(&platform, &mut daemon).await;
    platform
        .call(
            Method::PUT,
            "/settings/content-sources/steam-workshop",
            Some(json!({ "api_key": STEAM_KEY })),
        )
        .await;
    item(
        &platform,
        "3001",
        "Hydrocraft Core",
        &[],
        "Library.\nMod ID: HydrocraftCore",
    );
    item(
        &platform,
        "2001",
        "Hydrocraft",
        &["3001"],
        "Workshop ID: 2001\nMod ID: Hydrocraft\nMod ID: HydrocraftFood",
    );

    // La fiche propose les identifiants trouvés dans la description.
    let project = platform
        .call(
            Method::GET,
            &format!("/servers/{id}/content/mods/projects/2001"),
            None,
        )
        .await;
    assert_eq!(project.status, StatusCode::OK, "{}", project.json);
    assert_eq!(
        project.json["prefill"],
        json!({ "mod_ids": ["Hydrocraft", "HydrocraftFood"] })
    );

    let mods = format!("/servers/{id}/content/mods");
    // Télécharger sans rien activer ne servirait à rien : le prefab exige les mods à activer.
    let missing = platform
        .call(Method::POST, &mods, Some(json!({ "version_id": "2001" })))
        .await;
    assert_eq!(missing.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        missing.json["fields"][0]["field"], "values.mod_ids",
        "{}",
        missing.json
    );
    assert_eq!(missing.json["fields"][0]["code"], "content_value_required");

    let (installed, deploy) = tokio::join!(
        platform.call(
            Method::POST,
            &mods,
            Some(json!({ "version_id": "2001", "values": { "mod_ids": ["Hydrocraft"] } })),
        ),
        daemon.accept(),
    );
    assert_eq!(installed.status, StatusCode::ACCEPTED, "{}", installed.json);
    assert_eq!(installed.json.as_array().unwrap().len(), 2);
    let keys = ini_keys(&deploy);
    assert_eq!(keys["WorkshopItems"], "2001;3001");
    // Le contenu demandé garde la saisie ; sa dépendance reçoit ce que propose sa description.
    assert_eq!(keys["Mods"], r"\Hydrocraft;\HydrocraftCore");
    let dependency = installed
        .json
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["project_id"] == "3001")
        .unwrap();
    assert_eq!(
        dependency["values"],
        json!({ "mod_ids": ["HydrocraftCore"] })
    );
    assert_eq!(dependency["kind"], "reference");
    let event = platform.admin.last_audit("server.content_installed").await;
    assert_eq!(event.details["source"], "steam-workshop");
    assert_eq!(event.details["dependencies"], json!(["3001"]));

    // Une dépendance ne part pas seule.
    daemon.report(&id, "offline").await;
    platform.wait_for_state(&id, "offline").await;
    let refused = platform
        .call(
            Method::DELETE,
            &format!(
                "/servers/{id}/content/mods/{}",
                dependency["id"].as_str().unwrap()
            ),
            None,
        )
        .await;
    assert_eq!(refused.json["code"], "content_required");
}
