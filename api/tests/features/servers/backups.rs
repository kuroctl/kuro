//! Sauvegardes (ADR-0018) : création à chaud, fin rapportée par le nœud, rotation, verrou,
//! restauration, suppression, téléchargement par ticket, reprise après coupure, permissions.

use axum::body::Body;
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE, HOST, ORIGIN};
use axum::http::{Method, Request, StatusCode};
use base64::Engine;
use http_body_util::BodyExt;
use kuro_api::iam::domain::Permission;
use kuro_protocol::limits::BACKUP_CHUNK_BYTES;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

use crate::support::link::say_hello;
use crate::support::platform::{FakeDaemon, Platform};

/// Serveur Minecraft déployé et arrêté, avec `backup_limit` sauvegardes gardées.
async fn server(platform: &Platform, daemon: &mut FakeDaemon, backup_limit: i64) -> String {
    let (response, _) = tokio::join!(
        platform.call(
            Method::POST,
            "/servers",
            Some(json!({
                "name": "Survie",
                "node_id": daemon.node.id.as_uuid(),
                "prefab_id": "minecraft-java",
                "memory_mb": 2048,
                "cpu_percent": 200,
                "disk_mb": 10240,
                "backup_limit": backup_limit,
            })),
        ),
        daemon.accept(),
    );
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.json);
    let id = response.json["id"].as_str().unwrap().to_owned();
    daemon.report(&id, "offline").await;
    platform.wait_for_state(&id, "offline").await;
    id
}

/// Lance une sauvegarde acceptée par le nœud ; retourne la sauvegarde et la commande reçue.
async fn start_backup(platform: &Platform, daemon: &mut FakeDaemon, id: &str) -> (Value, Value) {
    let path = format!("/servers/{id}/backups");
    let (response, command) = tokio::join!(
        platform.call(Method::POST, &path, Some(json!({})),),
        daemon.accept_backup(),
    );
    assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.json);
    (response.json, command)
}

async fn finish(daemon: &mut FakeDaemon, id: &str, backup: &Value, outcome: Value) {
    let mut payload = json!({ "server_id": id, "backup_id": backup["id"] });
    payload
        .as_object_mut()
        .unwrap()
        .extend(outcome.as_object().unwrap().clone());
    daemon.event("backup.done", payload).await;
}

fn ok(size: u64, sha256: &str) -> Value {
    json!({ "status": "ok", "size": size, "sha256": sha256 })
}

const SHA: &str = "324fb9b3d3a59324cb5d96154f4fbc41c8f2d926f9bb2da1702f0230bd1b88d6";

/// Sauvegarde terminée avec succès.
async fn ready_backup(platform: &Platform, daemon: &mut FakeDaemon, id: &str) -> Value {
    let (backup, _) = start_backup(platform, daemon, id).await;
    finish(daemon, id, &backup, ok(1024, SHA)).await;
    wait_backup(platform, id, &backup, "ready").await
}

/// Attend qu'une sauvegarde atteigne `state` ; la retourne.
async fn wait_backup(platform: &Platform, id: &str, backup: &Value, state: &str) -> Value {
    for _ in 0..100 {
        let list = backups(platform, id).await;
        if let Some(found) = list["backups"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["id"] == backup["id"] && b["state"] == state)
        {
            return found.clone();
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("backup never became {state}");
}

async fn backups(platform: &Platform, id: &str) -> Value {
    let response = platform
        .call(Method::GET, &format!("/servers/{id}/backups"), None)
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    response.json
}

fn ids(list: &Value) -> Vec<Value> {
    list["backups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["id"].clone())
        .collect()
}

#[tokio::test]
async fn a_backup_runs_on_the_node_with_the_prefab_rules() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565").await;
    let id = server(&platform, &mut daemon, 3).await;

    let (backup, command) = start_backup(&platform, &mut daemon, &id).await;

    assert_eq!(backup["state"], "creating");
    assert_eq!(command["type"], "backup.create");
    assert_eq!(command["payload"]["server_id"], id);
    assert_eq!(command["payload"]["backup_id"], backup["id"]);
    assert_eq!(command["payload"]["ignore"], json!(["server/data/logs/**"]));
    assert_eq!(command["payload"]["before"][0]["console"], "save-off");
    assert_eq!(command["payload"]["after"][0]["console"], "save-on");
    let event = platform.admin.last_audit("server.backup_created").await;
    assert_eq!(event.target_id.as_deref(), Some(id.as_str()));
    assert_eq!(event.details["backup_id"], backup["id"]);

    finish(&mut daemon, &id, &backup, ok(1024, SHA)).await;
    let ready = wait_backup(&platform, &id, &backup, "ready").await;

    assert_eq!(ready["size_bytes"], 1024);
    assert_eq!(ready["sha256"], SHA);
    assert!(ready["completed_at"].is_string());
    assert_eq!(backups(&platform, &id).await["limit"], 3);
}

#[tokio::test]
async fn a_failed_backup_keeps_its_cause_and_frees_the_server() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565").await;
    let id = server(&platform, &mut daemon, 3).await;
    let (backup, _) = start_backup(&platform, &mut daemon, &id).await;

    let busy = platform
        .call(
            Method::POST,
            &format!("/servers/{id}/backups"),
            Some(json!({})),
        )
        .await;
    assert_eq!(busy.status, StatusCode::CONFLICT);
    assert_eq!(busy.json["code"], "backup_in_progress");

    finish(
        &mut daemon,
        &id,
        &backup,
        json!({ "status": "failed", "reason": "disk full" }),
    )
    .await;
    let failed = wait_backup(&platform, &id, &backup, "failed").await;

    assert_eq!(failed["failure"], "disk full");
    start_backup(&platform, &mut daemon, &id).await;
}

#[tokio::test]
async fn the_oldest_unlocked_backup_is_replaced_once_a_new_one_succeeds() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565").await;
    let id = server(&platform, &mut daemon, 2).await;
    let first = ready_backup(&platform, &mut daemon, &id).await;
    let second = ready_backup(&platform, &mut daemon, &id).await;
    let locked = platform
        .call(
            Method::PATCH,
            &format!("/servers/{id}/backups/{}", first["id"].as_str().unwrap()),
            Some(json!({ "locked": true })),
        )
        .await;
    assert_eq!(locked.status, StatusCode::OK, "{}", locked.json);

    let (third, _) = start_backup(&platform, &mut daemon, &id).await;
    // Rien n'est supprimé tant que la nouvelle sauvegarde n'a pas réussi.
    assert_eq!(ids(&backups(&platform, &id).await).len(), 3);
    finish(&mut daemon, &id, &third, ok(2048, SHA)).await;
    let deletion = daemon.accept_backup().await;

    assert_eq!(deletion["type"], "backup.delete");
    assert_eq!(deletion["payload"]["backup_id"], second["id"]);
    assert_eq!(
        ids(&backups(&platform, &id).await),
        [third["id"].clone(), first["id"].clone()]
    );
    let event = platform.admin.last_audit("server.backup_deleted").await;
    assert_eq!(event.actor_kind, "system");
    assert_eq!(event.details["cause"], "rotation");
}

#[tokio::test]
async fn locked_backups_or_a_zero_limit_refuse_new_backups() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25566").await;
    let id = server(&platform, &mut daemon, 1).await;
    let only = ready_backup(&platform, &mut daemon, &id).await;
    platform
        .call(
            Method::PATCH,
            &format!("/servers/{id}/backups/{}", only["id"].as_str().unwrap()),
            Some(json!({ "locked": true })),
        )
        .await;

    let full = platform
        .call(
            Method::POST,
            &format!("/servers/{id}/backups"),
            Some(json!({})),
        )
        .await;
    assert_eq!(full.status, StatusCode::CONFLICT);
    assert_eq!(full.json["code"], "backup_limit_reached");

    let disabled = server(&platform, &mut daemon, 0).await;
    let refused = platform
        .call(
            Method::POST,
            &format!("/servers/{disabled}/backups"),
            Some(json!({})),
        )
        .await;
    assert_eq!(refused.status, StatusCode::CONFLICT);
    assert_eq!(refused.json["code"], "backups_disabled");
}

#[tokio::test]
async fn a_restore_is_audited_and_blocks_the_server_meanwhile() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565").await;
    let id = server(&platform, &mut daemon, 3).await;
    let backup = ready_backup(&platform, &mut daemon, &id).await;
    let path = format!(
        "/servers/{id}/backups/{}/restore",
        backup["id"].as_str().unwrap()
    );

    let (accepted, command) = tokio::join!(
        platform.call(Method::POST, &path, Some(json!({ "mode": "clean" }))),
        daemon.accept_backup(),
    );

    assert_eq!(accepted.status, StatusCode::ACCEPTED, "{}", accepted.json);
    assert_eq!(command["type"], "backup.restore");
    assert_eq!(command["payload"]["mode"], "clean");
    let event = platform.admin.last_audit("server.backup_restored").await;
    assert_eq!(
        event.details,
        json!({ "backup_id": backup["id"], "mode": "clean" })
    );

    daemon.report(&id, "restoring").await;
    platform.wait_for_state(&id, "restoring").await;
    let power = platform
        .call(
            Method::POST,
            &format!("/servers/{id}/power"),
            Some(json!({ "action": "start" })),
        )
        .await;
    assert_eq!(power.status, StatusCode::CONFLICT);
    assert_eq!(power.json["code"], "server_busy");
}

#[tokio::test]
async fn only_unlocked_backups_can_be_deleted() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565").await;
    let id = server(&platform, &mut daemon, 3).await;
    let backup = ready_backup(&platform, &mut daemon, &id).await;
    let path = format!("/servers/{id}/backups/{}", backup["id"].as_str().unwrap());
    platform
        .call(Method::PATCH, &path, Some(json!({ "locked": true })))
        .await;

    let refused = platform.call(Method::DELETE, &path, None).await;
    assert_eq!(refused.status, StatusCode::CONFLICT);
    assert_eq!(refused.json["code"], "backup_locked");

    platform
        .call(Method::PATCH, &path, Some(json!({ "locked": false })))
        .await;
    let (deleted, command) = tokio::join!(
        platform.call(Method::DELETE, &path, None),
        daemon.accept_backup(),
    );
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.json);
    assert_eq!(command["type"], "backup.delete");
    assert!(ids(&backups(&platform, &id).await).is_empty());
    let event = platform.admin.last_audit("server.backup_deleted").await;
    assert_eq!(event.details["cause"], "admin");
}

/// Formulaire de téléchargement, comme le navigateur le soumet.
fn download_request(id: &str, backup: &Value, ticket: &str, origin: &str) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(format!(
            "/api/admin/v1/servers/{id}/backups/{}/download",
            backup["id"].as_str().unwrap()
        ))
        .header(HOST, "panel.example.com")
        .header(ORIGIN, origin)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(format!("ticket={ticket}")))
        .unwrap()
}

async fn ticket(platform: &Platform, id: &str, backup: &Value) -> String {
    let response = platform
        .call(
            Method::POST,
            &format!(
                "/servers/{id}/backups/{}/download/ticket",
                backup["id"].as_str().unwrap()
            ),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.json);
    response.json["ticket"].as_str().unwrap().to_owned()
}

/// Répond aux `backup.read` du téléchargement à partir de `archive`.
async fn serve_reads(daemon: &mut FakeDaemon, archive: &[u8]) {
    let mut served = 0;
    while served < archive.len() {
        let request = daemon.next_backup_request().await;
        assert_eq!(request["type"], "backup.read");
        let offset = usize::try_from(request["payload"]["offset"].as_u64().unwrap()).unwrap();
        let end = (offset + BACKUP_CHUNK_BYTES).min(archive.len());
        daemon
            .answer(
                &request,
                "backup.chunk",
                json!({
                    "data": base64::engine::general_purpose::STANDARD.encode(&archive[offset..end]),
                    "eof": end == archive.len(),
                }),
            )
            .await;
        served += end - offset;
    }
}

#[tokio::test]
async fn a_backup_downloads_through_a_single_use_ticket() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565").await;
    let id = server(&platform, &mut daemon, 3).await;
    let archive: Vec<u8> = (0..BACKUP_CHUNK_BYTES + 100)
        .map(|i| (i % 251) as u8)
        .collect();
    let sha: String = Sha256::digest(&archive)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let (backup, _) = start_backup(&platform, &mut daemon, &id).await;
    finish(&mut daemon, &id, &backup, ok(archive.len() as u64, &sha)).await;
    wait_backup(&platform, &id, &backup, "ready").await;
    let ticket = ticket(&platform, &id, &backup).await;
    let router = platform.admin.router();

    let request = download_request(&id, &backup, &ticket, "https://panel.example.com");
    let (body, ()) = tokio::join!(
        async {
            let response = router.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let headers = response.headers().clone();
            assert_eq!(headers[CONTENT_LENGTH], archive.len().to_string());
            assert!(
                headers[CONTENT_DISPOSITION]
                    .to_str()
                    .unwrap()
                    .starts_with("attachment; filename=\"Survie-")
            );
            response.into_body().collect().await.unwrap().to_bytes()
        },
        serve_reads(&mut daemon, &archive),
    );

    assert_eq!(body.as_ref(), archive.as_slice());
    let event = platform.admin.last_audit("server.backup_downloaded").await;
    assert_eq!(event.details["backup_id"], backup["id"]);
    // Le ticket ne sert qu'une fois.
    let again = router
        .clone()
        .oneshot(download_request(
            &id,
            &backup,
            &ticket,
            "https://panel.example.com",
        ))
        .await
        .unwrap();
    assert_eq!(again.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_download_from_another_origin_or_with_a_corrupt_archive_fails() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565").await;
    let id = server(&platform, &mut daemon, 3).await;
    let (backup, _) = start_backup(&platform, &mut daemon, &id).await;
    finish(&mut daemon, &id, &backup, ok(10, SHA)).await;
    wait_backup(&platform, &id, &backup, "ready").await;
    let router = platform.admin.router();

    let ticket_value = ticket(&platform, &id, &backup).await;
    let foreign = router
        .clone()
        .oneshot(download_request(
            &id,
            &backup,
            &ticket_value,
            "https://evil.example.com",
        ))
        .await
        .unwrap();
    assert_eq!(foreign.status(), StatusCode::FORBIDDEN);

    // Contenu qui ne correspond pas au sha256 enregistré : le transfert est coupé.
    let ticket_value = ticket(&platform, &id, &backup).await;
    let request = download_request(&id, &backup, &ticket_value, "https://panel.example.com");
    let (outcome, ()) = tokio::join!(
        async {
            let response = router.clone().oneshot(request).await.unwrap();
            response.into_body().collect().await
        },
        serve_reads(&mut daemon, b"0123456789"),
    );
    assert!(outcome.is_err());
}

#[tokio::test]
async fn backups_finished_during_a_disconnection_are_caught_up() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565").await;
    let id = server(&platform, &mut daemon, 3).await;
    let (backup, _) = start_backup(&platform, &mut daemon, &id).await;

    // Le lien tombe pendant l'archivage, puis le nœud revient.
    let node = daemon.node;
    drop(daemon.ws);
    let mut ws = platform.api.connect(&node).await;
    say_hello(&mut ws).await;
    let mut daemon = FakeDaemon { node, ws };
    let list = daemon.next_backup_request().await;
    assert_eq!(list["type"], "backup.list");
    assert_eq!(list["payload"]["server_id"], id);
    daemon
        .answer(
            &list,
            "backup.listing",
            json!({ "backups": [
                { "backup_id": backup["id"], "state": "ready", "size": 4096, "sha256": SHA }
            ] }),
        )
        .await;

    let ready = wait_backup(&platform, &id, &backup, "ready").await;
    assert_eq!(ready["size_bytes"], 4096);
}

#[tokio::test]
async fn a_backup_lost_by_the_node_is_marked_failed() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565").await;
    let id = server(&platform, &mut daemon, 3).await;
    let (backup, _) = start_backup(&platform, &mut daemon, &id).await;

    let node = daemon.node;
    drop(daemon.ws);
    let mut ws = platform.api.connect(&node).await;
    say_hello(&mut ws).await;
    let mut daemon = FakeDaemon { node, ws };
    let list = daemon.next_backup_request().await;
    daemon
        .answer(&list, "backup.listing", json!({ "backups": [] }))
        .await;

    let failed = wait_backup(&platform, &id, &backup, "failed").await;
    assert!(failed["failure"].as_str().unwrap().contains("interrupted"));
}

#[tokio::test]
async fn backups_require_their_permissions() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565").await;
    let id = server(&platform, &mut daemon, 3).await;
    let backup = ready_backup(&platform, &mut daemon, &id).await;
    let backup_id = backup["id"].as_str().unwrap();
    let admin = &platform.admin;
    let account = admin
        .account(
            "reader@example.com",
            &[Permission::ServersRead, Permission::ServersBackupsRead],
        )
        .await;
    let token = admin.login(&account).await.access_token;

    let listed = admin
        .call(
            Method::GET,
            &format!("/servers/{id}/backups"),
            Some(&token),
            None,
        )
        .await;
    assert_eq!(listed.status, StatusCode::OK);
    for (method, path, body, permission) in [
        (
            Method::POST,
            format!("/servers/{id}/backups"),
            Some(json!({})),
            "servers.backups.create",
        ),
        (
            Method::POST,
            format!("/servers/{id}/backups/{backup_id}/restore"),
            Some(json!({ "mode": "clean" })),
            "servers.backups.restore",
        ),
        (
            Method::DELETE,
            format!("/servers/{id}/backups/{backup_id}"),
            None,
            "servers.backups.delete",
        ),
        (
            Method::POST,
            format!("/servers/{id}/backups/{backup_id}/download/ticket"),
            None,
            "servers.backups.download",
        ),
    ] {
        let response = admin.call(method, &path, Some(&token), body).await;
        assert_eq!(response.status, StatusCode::FORBIDDEN, "{path}");
        assert_eq!(response.json["params"]["permission"], permission);
    }
}
