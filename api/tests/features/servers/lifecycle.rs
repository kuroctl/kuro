//! Cycle de vie d'un serveur vu de l'API : déploiement, état rapporté, réglages, actions,
//! suppression, resynchronisation.

use axum::http::{Method, StatusCode};
use serde_json::json;

use crate::support::link::recv;
use crate::support::platform::Platform;

#[tokio::test]
async fn a_server_is_deployed_with_a_resolved_spec() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;

    let (server, command) = platform
        .deploy_minecraft(&mut daemon, json!({ "motd": "Bienvenue" }))
        .await;

    assert_eq!(server["state"], "deploying");
    assert_eq!(
        server["prefab"],
        json!({ "id": "minecraft-java", "version": "1.4.0" })
    );
    assert_eq!(
        server["allocations"],
        json!([{ "name": "game", "ip": "0.0.0.0", "port": 25565 }])
    );
    // Le secret généré existe, mais sa valeur n'est jamais renvoyée.
    assert_eq!(server["secrets"], json!(["rcon_password"]));
    assert!(server["settings"].get("rcon_password").is_none());
    assert_eq!(server["settings"]["motd"], "Bienvenue");

    assert_eq!(command["type"], "server.deploy");
    let spec = &command["payload"];
    assert_eq!(spec["server_id"], server["id"]);
    let service = &spec["services"][0];
    assert_eq!(
        service["image"],
        "docker.io/itzg/minecraft-server:2026.10.1-java25"
    );
    assert_eq!(service["user"], "1000:1000");
    assert_eq!(service["env"]["MEMORY"], "2048M");
    assert_eq!(service["limits"]["cpu_millis"], 2000);
    assert_eq!(
        service["ports"][0]["published"],
        json!({ "host_ip": "0.0.0.0", "host_port": 25565 })
    );
    let properties = spec["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == "/data/server.properties")
        .unwrap();
    assert_eq!(properties["keys"]["motd"], "Bienvenue");
    let rcon = properties["keys"]["rcon.password"].as_str().unwrap();
    assert_eq!(rcon.len(), 32, "generated secret sent to the node only");

    let event = platform.admin.last_audit("server.created").await;
    assert!(
        !event.details.to_string().contains(rcon),
        "secret in the audit log"
    );

    daemon
        .report(server["id"].as_str().unwrap(), "offline")
        .await;
    platform
        .wait_for_state(server["id"].as_str().unwrap(), "offline")
        .await;
}

#[tokio::test]
async fn invalid_settings_are_reported_on_each_field() {
    let platform = Platform::start().await;
    let daemon = platform.online_node("node-1", "25565").await;

    let response = platform
        .call(
            Method::POST,
            "/servers",
            Some(json!({
                "name": "Survie",
                "node_id": daemon.node.id.as_uuid(),
                "prefab_id": "minecraft-java",
                "memory_mb": 2048,
                "cpu_percent": 100,
                "disk_mb": 1024,
                "settings": { "max_players": 0, "difficulty": "nightmare", "motd": "a\nb" },
            })),
        )
        .await;

    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(response.json["code"], "validation_failed");
    let mut fields: Vec<(String, String)> = response.json["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["field"].as_str().unwrap().to_owned(),
                f["params"]["reason"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    fields.sort();
    assert_eq!(
        fields,
        vec![
            ("settings.difficulty".to_owned(), "options".to_owned()),
            ("settings.max_players".to_owned(), "min".to_owned()),
            ("settings.motd".to_owned(), "control_character".to_owned()),
        ]
    );
}

#[tokio::test]
async fn deployment_needs_an_online_node_and_a_free_port() {
    let platform = Platform::start().await;
    let offline = platform.api.enroll_node("node-off").await;
    let mut daemon = platform.online_node("node-1", "25565").await;
    let body = |node: uuid::Uuid| {
        json!({
            "name": "Survie", "node_id": node, "prefab_id": "minecraft-java",
            "memory_mb": 1024, "cpu_percent": 100, "disk_mb": 1024,
        })
    };

    let refused = platform
        .call(Method::POST, "/servers", Some(body(offline.id.as_uuid())))
        .await;
    platform.deploy_minecraft(&mut daemon, json!({})).await;
    let full = platform
        .call(
            Method::POST,
            "/servers",
            Some(body(daemon.node.id.as_uuid())),
        )
        .await;

    assert_eq!(refused.status, StatusCode::CONFLICT);
    assert_eq!(refused.json["code"], "node_offline");
    assert_eq!(full.status, StatusCode::CONFLICT);
    assert_eq!(full.json["code"], "no_free_allocation");
}

#[tokio::test]
async fn a_refused_deployment_leaves_the_server_failed() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565").await;

    let (response, ()) = tokio::join!(
        platform.call(
            Method::POST,
            "/servers",
            Some(json!({
                "name": "Survie", "node_id": daemon.node.id.as_uuid(), "prefab_id": "minecraft-java",
                "memory_mb": 1024, "cpu_percent": 100, "disk_mb": 1024,
            })),
        ),
        async {
            let command = daemon.next_command().await;
            daemon
                .reply(&command, json!({ "status": "error", "code": "invalid_spec", "message": "runs as root" }))
                .await;
        },
    );

    assert_eq!(response.status, StatusCode::CREATED);
    assert_eq!(response.json["state"], "failed");
    assert_eq!(response.json["state_reason"], "daemon_error");
}

#[tokio::test]
async fn power_actions_are_forwarded_to_the_node() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565").await;
    let (server, _) = platform.deploy_minecraft(&mut daemon, json!({})).await;
    let id = server["id"].as_str().unwrap().to_owned();
    daemon.report(&id, "offline").await;
    platform.wait_for_state(&id, "offline").await;

    let power = format!("/servers/{id}/power");
    let (accepted, command) = tokio::join!(
        platform.call(Method::POST, &power, Some(json!({ "action": "start" }))),
        daemon.accept(),
    );
    let (busy, ()) = tokio::join!(
        platform.call(Method::POST, &power, Some(json!({ "action": "stop" }))),
        async {
            let command = daemon.next_command().await;
            daemon
                .reply(
                    &command,
                    json!({ "status": "error", "code": "busy", "message": "x" }),
                )
                .await;
        },
    );

    assert_eq!(accepted.status, StatusCode::ACCEPTED);
    assert_eq!(command["type"], "server.power");
    assert_eq!(
        command["payload"],
        json!({ "server_id": id, "action": "start" })
    );
    assert_eq!(busy.status, StatusCode::CONFLICT);
    assert_eq!(busy.json["code"], "server_busy");
    let event = platform.admin.last_audit("server.power").await;
    assert_eq!(event.details["action"], "start");

    daemon.report(&id, "running").await;
    platform.wait_for_state(&id, "running").await;
}

#[tokio::test]
async fn settings_changes_redeploy_and_respect_frozen_fields() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565").await;
    let (server, _) = platform
        .deploy_minecraft(&mut daemon, json!({ "loader": "fabric" }))
        .await;
    let id = server["id"].as_str().unwrap().to_owned();
    daemon.report(&id, "offline").await;
    platform.wait_for_state(&id, "offline").await;
    let path = format!("/servers/{id}/settings");

    let frozen = platform
        .call(
            Method::PATCH,
            &path,
            Some(json!({ "settings": { "loader": "forge" } })),
        )
        .await;
    let (updated, command) = tokio::join!(
        platform.call(
            Method::PATCH,
            &path,
            Some(json!({ "settings": { "max_players": 42 } }))
        ),
        daemon.accept(),
    );

    assert_eq!(frozen.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(frozen.json["fields"][0]["code"], "setting_not_editable");
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.json);
    assert_eq!(updated.json["settings"]["max_players"], 42);
    assert_eq!(command["type"], "server.deploy");
    let properties = command["payload"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == "/data/server.properties")
        .unwrap();
    assert_eq!(properties["keys"]["max-players"], "42");
    let event = platform.admin.last_audit("server.settings_updated").await;
    assert_eq!(event.details["changed"], json!(["max_players"]));
}

#[tokio::test]
async fn deletion_completes_when_the_node_confirms() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565").await;
    let (server, _) = platform.deploy_minecraft(&mut daemon, json!({})).await;
    let id = server["id"].as_str().unwrap().to_owned();
    daemon.report(&id, "offline").await;
    platform.wait_for_state(&id, "offline").await;

    let path = format!("/servers/{id}");
    let (accepted, command) =
        tokio::join!(platform.call(Method::DELETE, &path, None), daemon.accept(),);
    let deleting = platform
        .call(Method::GET, &format!("/servers/{id}"), None)
        .await;
    daemon.report(&id, "deleted").await;

    assert_eq!(accepted.status, StatusCode::ACCEPTED);
    assert_eq!(command["type"], "server.delete");
    assert_eq!(deleting.json["state"], "deleting");
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let gone = platform
            .call(Method::GET, &format!("/servers/{id}"), None)
            .await;
        if gone.status == StatusCode::NOT_FOUND {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "server not deleted: {}",
            gone.json
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let ports = platform
        .call(
            Method::GET,
            &format!("/nodes/{}/allocations", daemon.node.id),
            None,
        )
        .await;
    assert!(ports.json[0]["server_id"].is_null(), "port released");
}

#[tokio::test]
async fn a_reconnecting_node_receives_its_servers() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565").await;
    let (server, _) = platform.deploy_minecraft(&mut daemon, json!({})).await;

    let mut ws = platform.api.connect(&daemon.node).await;
    crate::support::link::send(
        &mut ws,
        crate::support::link::hello(
            &uuid::Uuid::now_v7().to_string(),
            kuro_protocol::PROTOCOL_VERSION,
        ),
    )
    .await;
    let ack = recv(&mut ws).await.unwrap();
    let sync = recv(&mut ws).await.unwrap();

    assert_eq!(ack["type"], "hello.ack");
    assert_eq!(sync["type"], "server.sync");
    assert_eq!(sync["payload"]["server_ids"], json!([server["id"]]));
}

#[tokio::test]
async fn a_node_hosting_servers_cannot_be_deleted() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565").await;
    platform.deploy_minecraft(&mut daemon, json!({})).await;
    let node = daemon.node.id;

    // Un nœud en ligne est refusé pour lui-même (ADR-0046) : hors ligne, ce sont ses serveurs.
    drop(daemon);
    platform.api.wait_for_status(node, "offline").await;
    let response = platform
        .call(Method::DELETE, &format!("/nodes/{node}"), None)
        .await;

    assert_eq!(response.status, StatusCode::CONFLICT);
    assert_eq!(response.json["code"], "node_has_servers");
}
