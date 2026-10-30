//! Ressources, quota disque et suspension (ADR-0019) : valeurs par défaut et plafonds du
//! projet, limites à chaud, espace mesuré par le nœud, refus d'un serveur suspendu.

use axum::http::{Method, StatusCode};
use kuro_api::iam::domain::Permission;
use serde_json::{Value, json};

use crate::support::platform::{FakeDaemon, Platform};

const MIB: u64 = 1024 * 1024;

/// Serveur Minecraft déployé et arrêté.
async fn offline_server(platform: &Platform, daemon: &mut FakeDaemon) -> String {
    let (server, _) = platform.deploy_minecraft(daemon, json!({})).await;
    let id = server["id"].as_str().unwrap().to_owned();
    daemon.report(&id, "offline").await;
    platform.wait_for_state(&id, "offline").await;
    id
}

async fn settings(platform: &Platform, body: Value) {
    let response = platform.call(Method::PATCH, "/settings", Some(body)).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
}

#[tokio::test]
async fn creation_takes_the_project_defaults_and_sends_the_disk_quota() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    settings(
        &platform,
        json!({ "default_memory_mb": 3072, "default_disk_mb": 5120, "default_backup_limit": 5 }),
    )
    .await;

    let (response, command) = tokio::join!(
        platform.call(
            Method::POST,
            "/servers",
            Some(json!({
                "name": "Survie",
                "node_id": daemon.node.id.as_uuid(),
                "prefab_id": "minecraft-java",
            })),
        ),
        daemon.accept(),
    );

    assert_eq!(response.status, StatusCode::CREATED, "{}", response.json);
    assert_eq!(response.json["memory_mb"], 3072);
    assert_eq!(response.json["cpu_percent"], 200);
    assert_eq!(response.json["disk_mb"], 5120);
    assert_eq!(response.json["backup_limit"], 5);
    assert_eq!(command["type"], "server.deploy");
    assert_eq!(command["payload"]["disk_bytes"], 5120 * MIB);
    assert!(command["payload"].get("suspended").is_none());
}

#[tokio::test]
async fn creation_above_a_ceiling_is_refused_before_reaching_the_node() {
    let platform = Platform::start().await;
    let daemon = platform.online_node("node-1", "25565-25570").await;
    settings(
        &platform,
        json!({ "max_memory_mb": 4096, "max_disk_mb": 20480 }),
    )
    .await;

    for (resources, field) in [
        (json!({ "memory_mb": 8192 }), "memory_mb"),
        // Illimité : au-delà de tout plafond.
        (json!({ "disk_mb": 0 }), "disk_mb"),
    ] {
        let mut body = json!({
            "name": "Survie",
            "node_id": daemon.node.id.as_uuid(),
            "prefab_id": "minecraft-java",
        });
        body.as_object_mut()
            .unwrap()
            .extend(resources.as_object().unwrap().clone());
        let response = platform.call(Method::POST, "/servers", Some(body)).await;

        assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY, "{field}");
        assert_eq!(response.json["code"], "resource_above_ceiling");
        assert_eq!(response.json["fields"][0]["field"], field);
    }
    let list = platform.call(Method::GET, "/servers", None).await;
    assert_eq!(list.json, json!([]));
}

#[tokio::test]
async fn resources_are_applied_live_then_saved_and_audited() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = offline_server(&platform, &mut daemon).await;

    let path = format!("/servers/{id}/resources");
    let (response, command) = tokio::join!(
        platform.call(
            Method::PATCH,
            &path,
            Some(json!({ "memory_mb": 4096, "disk_mb": 20480, "backup_limit": 5 })),
        ),
        daemon.accept(),
    );

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(response.json["memory_mb"], 4096);
    assert_eq!(response.json["backup_limit"], 5);
    assert_eq!(command["type"], "server.limits");
    assert_eq!(command["payload"]["disk_bytes"], 20480 * MIB);
    let limits = &command["payload"]["services"]["server"];
    assert_eq!(limits["memory_bytes"], 4096 * MIB);
    assert_eq!(limits["cpu_millis"], 2000);
    let event = platform.admin.last_audit("server.resources_updated").await;
    assert_eq!(event.target_id.as_deref(), Some(id.as_str()));
    assert_eq!(
        event.details["changed"],
        json!({
            "memory_mb": { "from": 2048, "to": 4096 },
            "disk_mb": { "from": 10240, "to": 20480 },
            "backup_limit": { "from": 3, "to": 5 },
        })
    );
}

#[tokio::test]
async fn a_refused_live_update_saves_nothing() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = offline_server(&platform, &mut daemon).await;

    let path = format!("/servers/{id}/resources");
    let (response, ()) = tokio::join!(
        platform.call(Method::PATCH, &path, Some(json!({ "memory_mb": 4096 }))),
        async {
            let command = daemon.next_command().await;
            daemon
                .reply(
                    &command,
                    json!({ "status": "error", "code": "busy", "message": "deploying" }),
                )
                .await;
        },
    );

    assert_eq!(response.status, StatusCode::CONFLICT);
    let server = platform
        .call(Method::GET, &format!("/servers/{id}"), None)
        .await;
    assert_eq!(server.json["memory_mb"], 2048);
}

#[tokio::test]
async fn only_changed_resources_meet_the_ceilings() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = offline_server(&platform, &mut daemon).await;
    // Plafond baissé sous la mémoire du serveur : il le garde.
    settings(
        &platform,
        json!({ "default_memory_mb": 1024, "max_memory_mb": 1024 }),
    )
    .await;
    let path = format!("/servers/{id}/resources");

    // Le nombre de sauvegardes ne touche pas le nœud.
    let backups = platform
        .call(Method::PATCH, &path, Some(json!({ "backup_limit": 1 })))
        .await;
    let memory = platform
        .call(Method::PATCH, &path, Some(json!({ "memory_mb": 3072 })))
        .await;
    let same = platform
        .call(Method::PATCH, &path, Some(json!({ "backup_limit": 1 })))
        .await;

    assert_eq!(backups.status, StatusCode::OK, "{}", backups.json);
    assert_eq!(backups.json["memory_mb"], 2048);
    assert_eq!(memory.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(memory.json["code"], "resource_above_ceiling");
    assert_eq!(memory.json["params"]["max"], 1024);
    assert_eq!(same.json["code"], "empty_patch");
}

#[tokio::test]
async fn the_node_reports_disk_usage_of_its_own_servers_only() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let mut other = platform.online_node("node-2", "25565-25570").await;
    let id = offline_server(&platform, &mut daemon).await;

    other
        .event("server.disk", json!({ "server_id": id, "used_bytes": 1 }))
        .await;
    daemon
        .event(
            "server.disk",
            json!({ "server_id": id, "used_bytes": 734_003_200 }),
        )
        .await;

    let path = format!("/servers/{id}");
    let mut server = Value::Null;
    for _ in 0..100 {
        server = platform.call(Method::GET, &path, None).await.json;
        if !server["disk_used_bytes"].is_null() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(server["disk_used_bytes"], 734_003_200);
    assert!(server["disk_measured_at"].is_string());
}

#[tokio::test]
async fn a_node_refusal_over_quota_has_its_own_code() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = offline_server(&platform, &mut daemon).await;

    let path = format!("/servers/{id}/power");
    let (response, ()) = tokio::join!(
        platform.call(Method::POST, &path, Some(json!({ "action": "start" }))),
        async {
            let command = daemon.next_command().await;
            daemon
                .reply(
                    &command,
                    json!({ "status": "error", "code": "disk_quota_exceeded", "message": "full" }),
                )
                .await;
        },
    );

    assert_eq!(response.status, StatusCode::CONFLICT);
    assert_eq!(response.json["code"], "disk_quota_exceeded");
}

#[tokio::test]
async fn a_suspended_server_neither_starts_nor_touches_its_data() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = offline_server(&platform, &mut daemon).await;
    let (suspend, unsuspend, power) = (
        format!("/servers/{id}/suspend"),
        format!("/servers/{id}/unsuspend"),
        format!("/servers/{id}/power"),
    );

    let (response, command) =
        tokio::join!(platform.call(Method::POST, &suspend, None), daemon.accept(),);
    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert!(response.json["suspended_at"].is_string());
    assert_eq!(command["type"], "server.suspend");
    assert_eq!(
        command["payload"],
        json!({ "server_id": id, "suspended": true })
    );
    let event = platform.admin.last_audit("server.suspended").await;
    assert_eq!(event.target_id.as_deref(), Some(id.as_str()));

    // Refusé par l'API, sans contacter le nœud.
    for (method, path, body) in [
        (
            Method::POST,
            format!("/servers/{id}/power"),
            Some(json!({ "action": "start" })),
        ),
        (
            Method::POST,
            format!("/servers/{id}/power"),
            Some(json!({ "action": "restart" })),
        ),
        (
            Method::POST,
            format!("/servers/{id}/command"),
            Some(json!({ "line": "say hi" })),
        ),
        (
            Method::GET,
            format!("/servers/{id}/files?path=/server/data"),
            None,
        ),
        (
            Method::POST,
            format!("/servers/{id}/backups"),
            Some(json!({})),
        ),
        (
            Method::POST,
            format!("/servers/{id}/reinstall"),
            Some(json!({ "mode": "reinstall" })),
        ),
    ] {
        let response = platform.call(method.clone(), &path, body).await;
        assert_eq!(response.status, StatusCode::CONFLICT, "{method} {path}");
        assert_eq!(response.json["code"], "server_suspended", "{method} {path}");
    }

    // L'arrêt reste possible.
    let (stop, _) = tokio::join!(
        platform.call(Method::POST, &power, Some(json!({ "action": "stop" }))),
        daemon.accept(),
    );
    assert_eq!(stop.status, StatusCode::ACCEPTED);

    let (reactivated, command) = tokio::join!(
        platform.call(Method::POST, &unsuspend, None),
        daemon.accept(),
    );
    assert_eq!(reactivated.status, StatusCode::OK);
    assert!(reactivated.json["suspended_at"].is_null());
    assert_eq!(command["payload"]["suspended"], false);
    platform.admin.last_audit("server.unsuspended").await;
}

#[tokio::test]
async fn a_redeploy_keeps_the_suspension() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = offline_server(&platform, &mut daemon).await;
    let (suspend, settings) = (
        format!("/servers/{id}/suspend"),
        format!("/servers/{id}/settings"),
    );
    let (_, _) = tokio::join!(platform.call(Method::POST, &suspend, None), daemon.accept(),);

    let (response, command) = tokio::join!(
        platform.call(
            Method::PATCH,
            &settings,
            Some(json!({ "settings": { "motd": "Fermé" } })),
        ),
        daemon.accept(),
    );

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(command["type"], "server.deploy");
    assert_eq!(command["payload"]["suspended"], true);
}

#[tokio::test]
async fn resources_and_suspension_need_their_permissions() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = offline_server(&platform, &mut daemon).await;
    let reader = platform
        .admin
        .account("reader@example.com", &[Permission::ServersRead])
        .await;
    let token = platform.admin.login(&reader).await.access_token;

    for (method, path, body) in [
        (
            Method::PATCH,
            format!("/servers/{id}/resources"),
            Some(json!({ "memory_mb": 4096 })),
        ),
        (Method::POST, format!("/servers/{id}/suspend"), None),
        (Method::POST, format!("/servers/{id}/unsuspend"), None),
    ] {
        let response = platform
            .admin
            .call(method.clone(), &path, Some(&token), body)
            .await;
        assert_eq!(response.status, StatusCode::FORBIDDEN, "{method} {path}");
    }
}

#[tokio::test]
async fn a_backup_over_the_measured_quota_is_refused_before_reaching_the_node() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = offline_server(&platform, &mut daemon).await;
    daemon
        .event(
            "server.disk",
            json!({ "server_id": id, "used_bytes": 10240 * MIB }),
        )
        .await;
    let path = format!("/servers/{id}");
    for _ in 0..100 {
        let server = platform.call(Method::GET, &path, None).await.json;
        if !server["disk_used_bytes"].is_null() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    let response = platform
        .call(
            Method::POST,
            &format!("/servers/{id}/backups"),
            Some(json!({})),
        )
        .await;

    assert_eq!(response.status, StatusCode::CONFLICT);
    assert_eq!(response.json["code"], "disk_quota_exceeded");
    let list = platform
        .call(Method::GET, &format!("/servers/{id}/backups"), None)
        .await;
    assert_eq!(list.json["backups"], json!([]));
}

#[tokio::test]
async fn a_backup_refused_by_the_node_is_audited_as_a_failure() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = offline_server(&platform, &mut daemon).await;

    let path = format!("/servers/{id}/backups");
    let (response, ()) = tokio::join!(platform.call(Method::POST, &path, Some(json!({}))), async {
        let request = daemon.next_backup_request().await;
        daemon
            .reply(
                &request,
                json!({ "status": "error", "code": "disk_quota_exceeded", "message": "full" }),
            )
            .await;
    },);

    assert_eq!(response.status, StatusCode::CONFLICT);
    assert_eq!(response.json["code"], "disk_quota_exceeded");
    let events: Vec<_> = platform
        .admin
        .audit()
        .await
        .into_iter()
        .filter(|row| row.action == "server.backup_created")
        .collect();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].outcome, "failure");
    assert_eq!(events[1].details["reason"], "disk_quota_exceeded");
    let list = platform.call(Method::GET, &path, None).await;
    assert_eq!(list.json["backups"][0]["state"], "failed");
}
