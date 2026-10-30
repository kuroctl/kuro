//! Installation et réinstallation (ADR-0017) : la première installation accompagne le
//! déploiement, un redéploiement ne réinstalle jamais, la réinstallation est auditée ; le
//! serveur peut démarrer de lui-même une fois installé.

use std::time::Duration;

use axum::http::{Method, StatusCode};
use kuro_api::iam::domain::Permission;
use serde_json::{Value, json};

use crate::support::platform::{FakeDaemon, Platform};

/// Crée un serveur Terraria ; le faux daemon accepte le déploiement, qui est retourné.
async fn create_terraria(platform: &Platform, daemon: &mut FakeDaemon) -> (String, Value) {
    create_terraria_starting(platform, daemon, false).await
}

async fn create_terraria_starting(
    platform: &Platform,
    daemon: &mut FakeDaemon,
    start_when_installed: bool,
) -> (String, Value) {
    let (response, command) = tokio::join!(
        platform.call(
            Method::POST,
            "/servers",
            Some(json!({
                "name": "Monde",
                "node_id": daemon.node.id.as_uuid(),
                "prefab_id": "terraria",
                "memory_mb": 1024,
                "cpu_percent": 100,
                "disk_mb": 4096,
                "settings": { "world_name": "Kuro" },
                "start_when_installed": start_when_installed,
            })),
        ),
        daemon.accept(),
    );
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.json);
    (response.json["id"].as_str().unwrap().to_owned(), command)
}

#[tokio::test]
async fn a_new_server_is_installed_with_its_deployment() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "7777").await;

    let (id, command) = create_terraria(&platform, &mut daemon).await;

    assert_eq!(command["type"], "server.deploy");
    let install = &command["payload"]["install"];
    assert_eq!(install["service"], "server");
    let steps = install["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 3);
    assert!(
        steps[0]["download"]["url"]
            .as_str()
            .unwrap()
            .starts_with("https://terraria.org/")
    );
    assert_eq!(steps[1]["extract"]["subdir"], "1449/Linux");
    assert_eq!(steps[2]["remove"]["path"], "/data/terraria-server.zip");

    // Le nœud installe, puis le serveur est prêt à démarrer.
    daemon.report(&id, "installing").await;
    platform.wait_for_state(&id, "installing").await;
    daemon.report(&id, "offline").await;
    platform.wait_for_state(&id, "offline").await;
}

#[tokio::test]
async fn a_prefab_that_installs_itself_sends_no_steps() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565").await;

    let (_, command) = platform.deploy_minecraft(&mut daemon, json!({})).await;

    assert_eq!(command["type"], "server.deploy");
    assert!(command["payload"].get("install").is_none());
}

#[tokio::test]
async fn changing_settings_never_reinstalls() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "7777").await;
    let (id, _) = create_terraria(&platform, &mut daemon).await;
    daemon.report(&id, "offline").await;
    platform.wait_for_state(&id, "offline").await;

    let path = format!("/servers/{id}/settings");
    let (updated, command) = tokio::join!(
        platform.call(
            Method::PATCH,
            &path,
            Some(json!({ "settings": { "max_players": 16 } })),
        ),
        daemon.accept(),
    );

    assert_eq!(updated.status, StatusCode::OK, "{}", updated.json);
    assert_eq!(command["type"], "server.deploy");
    assert!(command["payload"].get("install").is_none());
}

#[tokio::test]
async fn a_server_is_reinstalled_or_reset_on_request() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "7777").await;
    let (id, _) = create_terraria(&platform, &mut daemon).await;
    daemon.report(&id, "offline").await;
    platform.wait_for_state(&id, "offline").await;
    let path = format!("/servers/{id}/reinstall");

    for mode in ["reinstall", "reset"] {
        let (accepted, command) = tokio::join!(
            platform.call(Method::POST, &path, Some(json!({ "mode": mode }))),
            daemon.accept(),
        );
        assert_eq!(accepted.status, StatusCode::ACCEPTED, "{}", accepted.json);
        assert_eq!(command["type"], "server.install");
        assert_eq!(command["payload"]["server_id"], id);
        assert_eq!(command["payload"]["mode"], mode);
        assert_eq!(
            command["payload"]["install"]["steps"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        let event = platform.admin.last_audit("server.reinstalled").await;
        assert_eq!(event.target_id.as_deref(), Some(id.as_str()));
        assert_eq!(event.details, json!({ "mode": mode }));
    }
}

#[tokio::test]
async fn a_self_installing_prefab_can_still_be_reset() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565").await;
    let (server, _) = platform.deploy_minecraft(&mut daemon, json!({})).await;
    let id = server["id"].as_str().unwrap().to_owned();
    daemon.report(&id, "offline").await;
    platform.wait_for_state(&id, "offline").await;

    let path = format!("/servers/{id}/reinstall");
    let (accepted, command) = tokio::join!(
        platform.call(Method::POST, &path, Some(json!({ "mode": "reset" }))),
        daemon.accept(),
    );

    assert_eq!(accepted.status, StatusCode::ACCEPTED);
    assert_eq!(command["payload"]["mode"], "reset");
    // Volumes vidés seulement : l'image se réinstalle d'elle-même au démarrage.
    assert!(command["payload"].get("install").is_none());
}

#[tokio::test]
async fn nothing_else_runs_during_an_installation() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "7777").await;
    let (id, _) = create_terraria(&platform, &mut daemon).await;
    daemon.report(&id, "installing").await;
    platform.wait_for_state(&id, "installing").await;

    let reinstall = platform
        .call(
            Method::POST,
            &format!("/servers/{id}/reinstall"),
            Some(json!({ "mode": "reinstall" })),
        )
        .await;
    let power = platform
        .call(
            Method::POST,
            &format!("/servers/{id}/power"),
            Some(json!({ "action": "start" })),
        )
        .await;
    let write = platform
        .call(
            Method::PUT,
            &format!("/servers/{id}/files/content"),
            Some(json!({ "path": "/server/data/x.txt", "content": "x" })),
        )
        .await;

    for response in [reinstall, power, write] {
        assert_eq!(response.status, StatusCode::CONFLICT, "{}", response.json);
        assert_eq!(response.json["code"], "server_busy");
    }
}

#[tokio::test]
async fn reinstalling_requires_its_permission() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "7777").await;
    let (id, _) = create_terraria(&platform, &mut daemon).await;
    let admin = &platform.admin;
    let account = admin
        .account("operator@example.com", &[Permission::ServersPower])
        .await;
    let token = admin.login(&account).await.access_token;

    let response = admin
        .call(
            Method::POST,
            &format!("/servers/{id}/reinstall"),
            Some(&token),
            Some(json!({ "mode": "reset" })),
        )
        .await;

    assert_eq!(response.status, StatusCode::FORBIDDEN);
    assert_eq!(response.json["params"]["permission"], "servers.reinstall");
}

/// Attend que la demande de démarrage du serveur soit consommée (ou annulée) par l'API.
async fn wait_start_cleared(platform: &Platform, id: &str) {
    let id: uuid::Uuid = id.parse().unwrap();
    for _ in 0..100 {
        let pending: bool =
            sqlx::query_scalar("SELECT start_when_installed FROM servers WHERE id = $1")
                .bind(id)
                .fetch_one(&platform.api.pool)
                .await
                .unwrap();
        if !pending {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("start_when_installed stayed set");
}

async fn wait_audit(platform: &Platform, action: &str) -> crate::support::admin::AuditRow {
    for _ in 0..100 {
        if let Some(row) = platform
            .admin
            .audit()
            .await
            .into_iter()
            .rev()
            .find(|row| row.action == action)
        {
            return row;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no audit event {action}");
}

#[tokio::test]
async fn a_server_starts_once_installed_when_asked() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "7777").await;
    let (id, _) = create_terraria_starting(&platform, &mut daemon, true).await;
    let created = platform.admin.last_audit("server.created").await;
    assert_eq!(created.details["start_when_installed"], true);

    daemon.report(&id, "installing").await;
    daemon.report(&id, "offline").await;
    let command = daemon.accept().await;

    assert_eq!(command["type"], "server.power");
    assert_eq!(command["payload"]["server_id"], id);
    assert_eq!(command["payload"]["action"], "start");
    wait_start_cleared(&platform, &id).await;
    // L'audit est validé juste après la réponse du daemon : l'attendre.
    let event = wait_audit(&platform, "server.power").await;
    assert_eq!(event.actor_kind, "system");
    assert_eq!(event.target_id.as_deref(), Some(id.as_str()));
    assert_eq!(event.details, json!({ "action": "start" }));
}

#[tokio::test]
async fn a_failed_deployment_cancels_the_start() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "7777").await;
    let (id, _) = create_terraria_starting(&platform, &mut daemon, true).await;

    daemon.report(&id, "failed").await;
    wait_start_cleared(&platform, &id).await;
    daemon.report(&id, "offline").await;
    platform.wait_for_state(&id, "offline").await;

    // Aucun démarrage en attente : la prochaine commande reçue est celle de l'admin.
    let path = format!("/servers/{id}/reinstall");
    let (accepted, command) = tokio::join!(
        platform.call(Method::POST, &path, Some(json!({ "mode": "reinstall" }))),
        daemon.accept(),
    );
    assert_eq!(accepted.status, StatusCode::ACCEPTED, "{}", accepted.json);
    assert_eq!(command["type"], "server.install");
}

#[tokio::test]
async fn starting_once_installed_requires_the_power_permission() {
    let platform = Platform::start().await;
    let daemon = platform.online_node("node-1", "7777").await;
    let admin = &platform.admin;
    let account = admin
        .account("deployer@example.com", &[Permission::ServersCreate])
        .await;
    let token = admin.login(&account).await.access_token;

    let response = admin
        .call(
            Method::POST,
            "/servers",
            Some(&token),
            Some(json!({
                "name": "Monde",
                "node_id": daemon.node.id.as_uuid(),
                "prefab_id": "terraria",
                "memory_mb": 1024,
                "cpu_percent": 100,
                "disk_mb": 4096,
                "start_when_installed": true,
            })),
        )
        .await;

    assert_eq!(response.status, StatusCode::FORBIDDEN);
    assert_eq!(response.json["params"]["permission"], "servers.power");
}
