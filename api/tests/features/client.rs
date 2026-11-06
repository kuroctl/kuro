//! Espace client (ADR-0051, tranche 3) : un fournisseur d'identité d'extension, le propriétaire
//! qui ne voit et ne pilote que ses serveurs, les refus audités, le serveur suspendu ; sa console
//! et ses métriques (ADR-0054) ; ses fichiers (ADR-0055) ; ses sauvegardes et ses tâches
//! planifiées (ADR-0056) ; ses réglages et la réinstallation (ADR-0057) ; les membres d'un serveur
//! (ADR-0060).

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use axum::http::{Method, StatusCode};
use futures_util::future::BoxFuture;
use kuro_api::extension::{
    self, Extension, ExtensionError, ExtensionFactory, IdentityError, IdentityProvider,
};
use kuro_api::servers::domain::OwnerRef;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::admin::TestAdmin;
use crate::support::browser::{Browser, serve};
use crate::support::db;
use crate::support::http::{PANEL_ORIGIN, TestResponse};
use crate::support::platform::{FakeDaemon, Platform};

const ALICE: &str = "shop:alice";
const BOB: &str = "shop:bob";

/// Fournisseur de test : un jeton par propriétaire, `broken` simule une panne.
struct Tokens(HashMap<&'static str, &'static str>);

impl IdentityProvider for Tokens {
    fn authenticate<'a>(
        &'a self,
        token: &'a str,
    ) -> BoxFuture<'a, Result<Option<OwnerRef>, IdentityError>> {
        Box::pin(async move {
            if token == "broken" {
                return Err(IdentityError("backend down".to_owned()));
            }
            Ok(self.0.get(token).and_then(|owner| OwnerRef::parse(*owner)))
        })
    }
}

fn shop() -> ExtensionFactory {
    ExtensionFactory::new("shop", |_| {
        let tokens = Tokens(HashMap::from([("alice-token", ALICE), ("bob-token", BOB)]));
        Ok(Extension::default().identity(Arc::new(tokens)))
    })
}

async fn client(
    platform: &Platform,
    method: Method,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> TestResponse {
    let authorization = token.map(|token| format!("Bearer {token}"));
    let headers: Vec<(&str, &str)> = authorization
        .as_deref()
        .map(|value| vec![("authorization", value)])
        .unwrap_or_default();
    platform
        .admin
        .raw(
            method,
            &format!("/api/client/v1{path}"),
            &headers,
            body.map(|body| ("application/json", body.to_string())),
        )
        .await
}

/// Serveur déployé pour `owner` (ou interne), arrêté.
async fn server_of(platform: &Platform, daemon: &mut FakeDaemon, owner: Value) -> String {
    let (response, _) = tokio::join!(
        platform.call(
            Method::POST,
            "/servers",
            Some(json!({
                "name": "Survie",
                "node_id": daemon.node.id.as_uuid(),
                "prefab_id": "minecraft-java",
                "owner_ref": owner,
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

#[tokio::test]
async fn without_identity_provider_there_is_no_client_space() {
    let kuro = TestAdmin::start().await;

    let response = kuro
        .raw(
            Method::GET,
            "/api/client/v1/servers",
            &[("authorization", "Bearer x")],
            None,
        )
        .await;

    assert_eq!(response.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_missing_or_unknown_token_is_unauthorized() {
    let platform = Platform::start_with_extensions(vec![shop()]).await;

    for token in [None, Some("nope"), Some("")] {
        let response = client(&platform, Method::GET, "/servers", token, None).await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED, "{token:?}");
        assert_eq!(response.json["code"], "unauthorized");
    }
}

#[tokio::test]
async fn an_admin_token_does_not_open_the_client_space() {
    let platform = Platform::start_with_extensions(vec![shop()]).await;

    let token = platform.token.clone();
    let response = client(&platform, Method::GET, "/servers", Some(&token), None).await;

    assert_eq!(response.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_failing_provider_is_an_internal_error() {
    let platform = Platform::start_with_extensions(vec![shop()]).await;

    let response = client(&platform, Method::GET, "/servers", Some("broken"), None).await;

    assert_eq!(response.status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn an_owner_lists_only_their_servers() {
    let platform = Platform::start_with_extensions(vec![shop()]).await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let alice = server_of(&platform, &mut daemon, json!(ALICE)).await;
    server_of(&platform, &mut daemon, json!(BOB)).await;
    server_of(&platform, &mut daemon, Value::Null).await;

    let response = client(
        &platform,
        Method::GET,
        "/servers",
        Some("alice-token"),
        None,
    )
    .await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    let servers = response.json.as_array().unwrap();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0]["id"], alice.as_str());
    assert_eq!(servers[0]["suspended"], false);
    assert_eq!(servers[0]["allocations"][0]["name"], "game");
    // Vue client : ni nœud, ni réglages, ni propriétaire.
    for hidden in ["node", "settings", "secrets", "owner_ref", "state_reason"] {
        assert!(servers[0].get(hidden).is_none(), "{hidden}");
    }
}

#[tokio::test]
async fn another_owners_server_looks_missing_and_the_attempt_is_audited() {
    let platform = Platform::start_with_extensions(vec![shop()]).await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let bob = server_of(&platform, &mut daemon, json!(BOB)).await;

    let read = client(
        &platform,
        Method::GET,
        &format!("/servers/{bob}"),
        Some("alice-token"),
        None,
    )
    .await;
    let unknown = client(
        &platform,
        Method::GET,
        &format!("/servers/{}", Uuid::now_v7()),
        Some("alice-token"),
        None,
    )
    .await;

    assert_eq!(read.status, StatusCode::NOT_FOUND);
    assert_eq!(read.json["code"], unknown.json["code"]);
    let denied = platform.admin.last_audit("access.denied").await;
    assert_eq!(denied.outcome, "denied");
    assert_eq!(denied.actor_kind, "owner");
    assert_eq!(denied.actor_id, None);
    assert_eq!(denied.target_id.as_deref(), Some(bob.as_str()));
    assert_eq!(
        denied.details,
        json!({ "reason": "not_owner", "owner_ref": ALICE })
    );
}

#[tokio::test]
async fn an_owner_cannot_power_another_owners_server() {
    let platform = Platform::start_with_extensions(vec![shop()]).await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let bob = server_of(&platform, &mut daemon, json!(BOB)).await;

    let response = client(
        &platform,
        Method::POST,
        &format!("/servers/{bob}/power"),
        Some("alice-token"),
        Some(json!({ "action": "start" })),
    )
    .await;

    assert_eq!(response.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_owner_powers_their_server_and_is_the_audited_actor() {
    let platform = Platform::start_with_extensions(vec![shop()]).await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let alice = server_of(&platform, &mut daemon, json!(ALICE)).await;

    let power = format!("/servers/{alice}/power");
    let (response, command) = tokio::join!(
        client(
            &platform,
            Method::POST,
            &power,
            Some("alice-token"),
            Some(json!({ "action": "start" })),
        ),
        daemon.accept(),
    );

    assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.json);
    assert_eq!(command["type"], "server.power");
    assert_eq!(
        command["payload"],
        json!({ "server_id": alice, "action": "start" })
    );
    let event = platform.admin.last_audit("server.power").await;
    assert_eq!(event.actor_kind, "owner");
    assert_eq!(event.details["owner_ref"], ALICE);
    assert_eq!(event.details["action"], "start");
}

#[tokio::test]
async fn a_suspended_server_refuses_every_power_action_of_its_owner() {
    let platform = Platform::start_with_extensions(vec![shop()]).await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let alice = server_of(&platform, &mut daemon, json!(ALICE)).await;
    let suspend = format!("/servers/{alice}/suspend");
    let (suspended, _) =
        tokio::join!(platform.call(Method::POST, &suspend, None), daemon.accept(),);
    assert_eq!(suspended.status, StatusCode::OK, "{}", suspended.json);

    for action in ["start", "stop", "restart", "kill"] {
        let response = client(
            &platform,
            Method::POST,
            &format!("/servers/{alice}/power"),
            Some("alice-token"),
            Some(json!({ "action": action })),
        )
        .await;
        assert_eq!(response.status, StatusCode::CONFLICT, "{action}");
        assert_eq!(response.json["code"], "server_suspended");
    }
    let read = client(
        &platform,
        Method::GET,
        &format!("/servers/{alice}"),
        Some("alice-token"),
        None,
    )
    .await;
    assert_eq!(read.json["suspended"], true);
}

#[tokio::test]
async fn two_extensions_cannot_both_provide_the_identity() {
    let pool = db::fresh_pool().await;
    let other = ExtensionFactory::new("other", |_| {
        Ok(Extension::default().identity(Arc::new(Tokens(HashMap::new()))))
    });

    let result = extension::prepare(&pool, &BTreeMap::new(), vec![shop(), other]).await;

    assert!(matches!(
        result,
        Err(ExtensionError::IdentityTwice { first, second }) if first == "shop" && second == "other"
    ));
}

/// Serveur de `owner` en marche, et le REST servi sur un vrai port (WebSocket de console).
async fn running_server_of(
    platform: &Platform,
    daemon: &mut FakeDaemon,
    owner: &str,
) -> (String, std::net::SocketAddr) {
    let id = server_of(platform, daemon, json!(owner)).await;
    daemon.report(&id, "running").await;
    platform.wait_for_state(&id, "running").await;
    (id, serve(platform.admin.router()).await)
}

#[tokio::test]
async fn an_owner_opens_the_console_of_their_server_and_sends_a_command() {
    let platform = Platform::start_with_extensions(vec![shop()]).await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let (alice, addr) = running_server_of(&platform, &mut daemon, ALICE).await;

    let ticket = client(
        &platform,
        Method::POST,
        &format!("/servers/{alice}/console/ticket"),
        Some("alice-token"),
        None,
    )
    .await;
    assert_eq!(ticket.status, StatusCode::CREATED, "{}", ticket.json);
    assert_eq!(ticket.json["expires_in_secs"], 30);
    let opened = platform.admin.last_audit("server.console_opened").await;
    assert_eq!(opened.actor_kind, "owner");
    assert_eq!(opened.details["owner_ref"], ALICE);

    let mut browser = Browser::open_at(addr, "client", &alice, PANEL_ORIGIN)
        .await
        .unwrap();
    browser.auth(ticket.json["ticket"].as_str().unwrap()).await;
    assert_eq!(browser.next_of("state").await["state"], "running");
    assert_eq!(daemon.accept().await["type"], "server.watch");

    let command_path = format!("/servers/{alice}/command");
    let (sent, command) = tokio::join!(
        client(
            &platform,
            Method::POST,
            &command_path,
            Some("alice-token"),
            Some(json!({ "line": "say Bonjour" })),
        ),
        daemon.accept(),
    );
    assert_eq!(sent.status, StatusCode::ACCEPTED, "{}", sent.json);
    assert_eq!(command["type"], "server.command");
    assert_eq!(command["payload"]["line"], "say Bonjour");
    let audited = platform.admin.last_audit("server.command").await;
    assert_eq!(audited.actor_kind, "owner");
    browser.close().await;
}

#[tokio::test]
async fn an_owner_reaches_neither_the_console_nor_the_metrics_of_another_owner() {
    let platform = Platform::start_with_extensions(vec![shop()]).await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let (bob, _) = running_server_of(&platform, &mut daemon, BOB).await;

    for (method, path, body) in [
        (Method::POST, format!("/servers/{bob}/console/ticket"), None),
        (
            Method::POST,
            format!("/servers/{bob}/command"),
            Some(json!({ "line": "stop" })),
        ),
        (Method::GET, format!("/servers/{bob}/metrics"), None),
    ] {
        let response = client(&platform, method, &path, Some("alice-token"), body).await;
        assert_eq!(response.status, StatusCode::NOT_FOUND, "{path}");
    }
}

#[tokio::test]
async fn an_owner_reads_the_metrics_of_their_server() {
    let platform = Platform::start_with_extensions(vec![shop()]).await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let alice = server_of(&platform, &mut daemon, json!(ALICE)).await;

    let metrics = client(
        &platform,
        Method::GET,
        &format!("/servers/{alice}/metrics?range=7d"),
        Some("alice-token"),
        None,
    )
    .await;
    assert_eq!(metrics.status, StatusCode::OK, "{}", metrics.json);
    assert_eq!(metrics.json["range"], "7d");
    assert_eq!(metrics.json["step_secs"], 3600);
    let wrong = client(
        &platform,
        Method::GET,
        &format!("/servers/{alice}/metrics?range=1y"),
        Some("alice-token"),
        None,
    )
    .await;
    assert_eq!(wrong.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn a_suspended_server_gives_its_owner_no_console() {
    let platform = Platform::start_with_extensions(vec![shop()]).await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let alice = server_of(&platform, &mut daemon, json!(ALICE)).await;
    let suspend = format!("/servers/{alice}/suspend");
    let (suspended, _) = tokio::join!(platform.call(Method::POST, &suspend, None), daemon.accept());
    assert_eq!(suspended.status, StatusCode::OK, "{}", suspended.json);

    let ticket = client(
        &platform,
        Method::POST,
        &format!("/servers/{alice}/console/ticket"),
        Some("alice-token"),
        None,
    )
    .await;

    assert_eq!(ticket.status, StatusCode::CONFLICT);
    assert_eq!(ticket.json["code"], "server_suspended");
}

/// Appel du propriétaire pendant que le nœud répond `kind` / `payload` à la requête de fichier.
async fn file_exchange(
    platform: &Platform,
    daemon: &mut FakeDaemon,
    method: Method,
    path: &str,
    body: Option<Value>,
    kind: &str,
    payload: Value,
) -> (TestResponse, Value) {
    tokio::join!(
        client(platform, method, path, Some("alice-token"), body),
        async {
            let request = daemon.next_file_request().await;
            daemon.answer(&request, kind, payload).await;
            request
        }
    )
}

#[tokio::test]
async fn an_owner_browses_reads_and_writes_the_files_of_their_server() {
    let platform = Platform::start_with_extensions(vec![shop()]).await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let (alice, _) = running_server_of(&platform, &mut daemon, ALICE).await;

    let (listed, request) = file_exchange(
        &platform,
        &mut daemon,
        Method::GET,
        &format!("/servers/{alice}/files?path=/server/data"),
        None,
        "file.listing",
        json!({ "entries": [{ "name": "server.properties", "kind": "file", "size": 10, "modified_ms": 1000 }] }),
    )
    .await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.json);
    assert_eq!(request["type"], "file.list");
    assert_eq!(listed.json["entries"][0]["name"], "server.properties");

    let (read, _) = file_exchange(
        &platform,
        &mut daemon,
        Method::GET,
        &format!("/servers/{alice}/files/content?path=/server/data/server.properties"),
        None,
        "file.content",
        json!({ "content": "motd=Kuro\n", "size": 10, "modified_ms": 1000, "sha256": "ab12" }),
    )
    .await;
    assert_eq!(read.status, StatusCode::OK, "{}", read.json);
    assert_eq!(read.json["sha256"], "ab12");

    let (written, request) = file_exchange(
        &platform,
        &mut daemon,
        Method::PUT,
        &format!("/servers/{alice}/files/content"),
        Some(json!({ "path": "/server/data/server.properties", "content": "motd=Alice\n", "if_sha256": "ab12" })),
        "command.result",
        json!({ "status": "ok" }),
    )
    .await;
    assert_eq!(written.status, StatusCode::NO_CONTENT, "{}", written.json);
    assert_eq!(request["payload"]["if_sha256"], "ab12");
    // Historique : un client, sans identifiant (le propriétaire n'a pas d'uuid).
    assert_eq!(request["payload"]["author"]["kind"], "customer");
    assert!(request["payload"]["author"].get("id").is_none());
    let event = platform.admin.last_audit("server.file_written").await;
    assert_eq!(event.actor_kind, "owner");
    assert_eq!(event.details["owner_ref"], ALICE);
}

#[tokio::test]
async fn an_owner_reaches_no_file_of_another_owner() {
    let platform = Platform::start_with_extensions(vec![shop()]).await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let (bob, _) = running_server_of(&platform, &mut daemon, BOB).await;

    for (method, path, body) in [
        (Method::GET, format!("/servers/{bob}/files?path=/"), None),
        (
            Method::GET,
            format!("/servers/{bob}/files/content?path=/server/data/a.txt"),
            None,
        ),
        (
            Method::PUT,
            format!("/servers/{bob}/files/content"),
            Some(json!({ "path": "/server/data/a.txt", "content": "x" })),
        ),
        (
            Method::DELETE,
            format!("/servers/{bob}/files?path=/server/data/a.txt"),
            None,
        ),
        (
            Method::GET,
            format!("/servers/{bob}/files/history?path=/server/data/a.txt"),
            None,
        ),
    ] {
        let response = client(&platform, method, &path, Some("alice-token"), body).await;
        assert_eq!(response.status, StatusCode::NOT_FOUND, "{path}");
    }
}

#[tokio::test]
async fn a_suspended_server_gives_its_owner_no_file() {
    let platform = Platform::start_with_extensions(vec![shop()]).await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let alice = server_of(&platform, &mut daemon, json!(ALICE)).await;
    let suspend = format!("/servers/{alice}/suspend");
    let (suspended, _) = tokio::join!(platform.call(Method::POST, &suspend, None), daemon.accept());
    assert_eq!(suspended.status, StatusCode::OK, "{}", suspended.json);

    let listed = client(
        &platform,
        Method::GET,
        &format!("/servers/{alice}/files?path=/"),
        Some("alice-token"),
        None,
    )
    .await;

    assert_eq!(listed.status, StatusCode::CONFLICT);
    assert_eq!(listed.json["code"], "server_suspended");
}

#[tokio::test]
async fn an_owner_backs_up_their_server_and_lists_the_backups() {
    let platform = Platform::start_with_extensions(vec![shop()]).await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let alice = server_of(&platform, &mut daemon, json!(ALICE)).await;
    let backups = format!("/servers/{alice}/backups");

    let (created, command) = tokio::join!(
        client(
            &platform,
            Method::POST,
            &backups,
            Some("alice-token"),
            Some(json!({ "name": "Avant la mise à jour" })),
        ),
        daemon.accept_backup(),
    );
    assert_eq!(created.status, StatusCode::ACCEPTED, "{}", created.json);
    assert_eq!(created.json["state"], "creating");
    assert_eq!(command["type"], "backup.create");
    let event = platform.admin.last_audit("server.backup_created").await;
    assert_eq!(event.actor_kind, "owner");
    assert_eq!(event.details["owner_ref"], ALICE);

    let listed = client(&platform, Method::GET, &backups, Some("alice-token"), None).await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.json);
    assert_eq!(listed.json["backups"][0]["name"], "Avant la mise à jour");
    assert!(listed.json["limit"].is_number());

    let bob = client(&platform, Method::GET, &backups, Some("bob-token"), None).await;
    assert_eq!(bob.status, StatusCode::NOT_FOUND);
}

fn schedule(action: &str, value: Option<&str>) -> Value {
    json!({
        "name": "Nuit",
        "cron": "0 4 * * *",
        "timezone": "Europe/Paris",
        "steps": [{ "action": action, "value": value }],
    })
}

#[tokio::test]
async fn an_owner_schedules_only_what_the_client_space_allows() {
    let platform = Platform::start_with_extensions(vec![shop()]).await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let alice = server_of(&platform, &mut daemon, json!(ALICE)).await;
    let schedules = format!("/servers/{alice}/schedules");

    let created = client(
        &platform,
        Method::POST,
        &schedules,
        Some("alice-token"),
        Some(schedule("backup", None)),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json);
    assert_eq!(created.json["enabled"], true);
    assert!(created.json["next_run_at"].is_string());

    // Action du prefab, mise à jour : pas (encore) dans l'espace client.
    for (action, value) in [("action", Some("whitelist")), ("upgrade", None)] {
        let refused = client(
            &platform,
            Method::POST,
            &schedules,
            Some("alice-token"),
            Some(schedule(action, value)),
        )
        .await;
        assert_eq!(refused.status, StatusCode::FORBIDDEN, "{action}");
    }

    let id = created.json["id"].as_str().unwrap();
    let mut disabled = schedule("power", Some("restart"));
    disabled["enabled"] = json!(false);
    let updated = client(
        &platform,
        Method::PUT,
        &format!("{schedules}/{id}"),
        Some("alice-token"),
        Some(disabled),
    )
    .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.json);
    assert_eq!(updated.json["enabled"], false);
    assert_eq!(updated.json["steps"][0]["value"], "restart");

    let listed = client(
        &platform,
        Method::GET,
        &schedules,
        Some("alice-token"),
        None,
    )
    .await;
    assert_eq!(listed.json.as_array().unwrap().len(), 1);
    let bob = client(&platform, Method::GET, &schedules, Some("bob-token"), None).await;
    assert_eq!(bob.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_owner_sees_and_changes_only_the_settings_open_to_customers() {
    let platform = Platform::start_with_extensions(vec![shop()]).await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let alice = server_of(&platform, &mut daemon, json!(ALICE)).await;
    let settings = format!("/servers/{alice}/settings");

    let read = client(
        &platform,
        Method::GET,
        &format!("{settings}?lang=fr"),
        Some("alice-token"),
        None,
    )
    .await;
    assert_eq!(read.status, StatusCode::OK, "{}", read.json);
    let fields: Vec<&Value> = read.json["groups"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|group| group["fields"].as_array().unwrap())
        .collect();
    let keys: Vec<&str> = fields.iter().map(|f| f["key"].as_str().unwrap()).collect();
    assert!(
        keys.contains(&"motd") && keys.contains(&"difficulty"),
        "{keys:?}"
    );
    // Fixés par la formule, secrets, mode hors ligne : jamais pour le client.
    for hidden in ["max_players", "rcon_password", "online_mode"] {
        assert!(!keys.contains(&hidden), "{hidden}");
        assert!(read.json["values"].get(hidden).is_none(), "{hidden}");
    }
    let motd = fields.iter().find(|f| f["key"] == "motd").unwrap();
    assert_eq!(motd["label"], "Message d'accueil");
    let difficulty = fields.iter().find(|f| f["key"] == "difficulty").unwrap();
    assert_eq!(
        difficulty["options"][3],
        json!({ "value": "hard", "label": "Difficile" })
    );
    let seed = fields.iter().find(|f| f["key"] == "level_seed").unwrap();
    assert_eq!(seed["editable"], false);
    assert_eq!(read.json["values"]["motd"], "A Kuro server");

    let (changed, _) = tokio::join!(
        client(
            &platform,
            Method::PATCH,
            &settings,
            Some("alice-token"),
            Some(json!({ "settings": { "motd": "Chez Alice" } })),
        ),
        daemon.accept(),
    );
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.json);
    assert_eq!(changed.json["values"]["motd"], "Chez Alice");
    let event = platform.admin.last_audit("server.settings_updated").await;
    assert_eq!(event.actor_kind, "owner");
    assert_eq!(event.details["changed"], json!(["motd"]));

    // Hors de sa vue, ou fixé au déploiement : refusé, rien n'est envoyé au nœud.
    for (key, value) in [("max_players", json!(200)), ("level_seed", json!("autre"))] {
        let refused = client(
            &platform,
            Method::PATCH,
            &settings,
            Some("alice-token"),
            Some(json!({ "settings": { key: value } })),
        )
        .await;
        assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY, "{key}");
        assert_eq!(
            refused.json["fields"][0]["field"],
            format!("settings.{key}"),
            "{}",
            refused.json
        );
        assert_eq!(refused.json["fields"][0]["code"], "setting_not_editable");
    }

    let bob = client(&platform, Method::GET, &settings, Some("bob-token"), None).await;
    assert_eq!(bob.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_owner_reinstalls_their_server_keeping_its_data() {
    let platform = Platform::start_with_extensions(vec![shop()]).await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let alice = server_of(&platform, &mut daemon, json!(ALICE)).await;
    let reinstall = format!("/servers/{alice}/reinstall");

    let (accepted, command) = tokio::join!(
        client(
            &platform,
            Method::POST,
            &reinstall,
            Some("alice-token"),
            None
        ),
        daemon.accept(),
    );

    assert_eq!(accepted.status, StatusCode::ACCEPTED, "{}", accepted.json);
    assert_eq!(command["type"], "server.install");
    assert_eq!(command["payload"]["mode"], "reinstall");
    let event = platform.admin.last_audit("server.reinstalled").await;
    assert_eq!(event.actor_kind, "owner");
    assert_eq!(event.details["mode"], "reinstall");
    let bob = client(&platform, Method::POST, &reinstall, Some("bob-token"), None).await;
    assert_eq!(bob.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_owner_renames_their_server() {
    let platform = Platform::start_with_extensions(vec![shop()]).await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let alice = server_of(&platform, &mut daemon, json!(ALICE)).await;
    let path = format!("/servers/{alice}");

    let renamed = client(
        &platform,
        Method::PATCH,
        &path,
        Some("alice-token"),
        Some(json!({ "name": "  Survie entre potes " })),
    )
    .await;
    assert_eq!(renamed.status, StatusCode::OK, "{}", renamed.json);
    assert_eq!(renamed.json["name"], "Survie entre potes");
    let event = platform.admin.last_audit("server.renamed").await;
    assert_eq!(event.actor_kind, "owner");

    let empty = client(
        &platform,
        Method::PATCH,
        &path,
        Some("alice-token"),
        Some(json!({ "name": " " })),
    )
    .await;
    assert_eq!(empty.status, StatusCode::UNPROCESSABLE_ENTITY);
    let bob = client(
        &platform,
        Method::PATCH,
        &path,
        Some("bob-token"),
        Some(json!({ "name": "Volé" })),
    )
    .await;
    assert_eq!(bob.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_member_manages_the_server_like_its_owner_until_removed() {
    let platform = Platform::start_with_extensions(vec![shop()]).await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let alice = server_of(&platform, &mut daemon, json!(ALICE)).await;
    let id: Uuid = alice.parse().unwrap();
    let state = &platform.admin.servers;
    let system = kuro_api::audit::domain::AuditContext::system();

    kuro_api::servers::application::add_member(state, &system, id, BOB)
        .await
        .unwrap();
    // Ajouté deux fois : sans effet.
    kuro_api::servers::application::add_member(state, &system, id, BOB)
        .await
        .unwrap();

    let listed = client(&platform, Method::GET, "/servers", Some("bob-token"), None).await;
    assert_eq!(listed.json.as_array().unwrap().len(), 1, "{}", listed.json);
    assert_eq!(listed.json[0]["id"], alice.as_str());
    let path = format!("/servers/{alice}/power");
    let (powered, command) = tokio::join!(
        client(
            &platform,
            Method::POST,
            &path,
            Some("bob-token"),
            Some(json!({ "action": "start" })),
        ),
        daemon.accept(),
    );
    assert_eq!(powered.status, StatusCode::ACCEPTED, "{}", powered.json);
    assert_eq!(command["type"], "server.power");
    let power = platform.admin.last_audit("server.power").await;
    assert_eq!(
        power.details["owner_ref"], BOB,
        "the member is the audited actor"
    );
    let added = platform.admin.last_audit("server.member_added").await;
    assert_eq!(added.details["member"], BOB);

    kuro_api::servers::application::remove_member(state, &system, id, BOB)
        .await
        .unwrap();

    let gone = client(
        &platform,
        Method::GET,
        &format!("/servers/{alice}"),
        Some("bob-token"),
        None,
    )
    .await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
    let removed = platform.admin.last_audit("server.member_removed").await;
    assert_eq!(removed.details["member"], BOB);
}

#[tokio::test]
async fn an_owner_searches_installs_and_removes_mods_on_their_server_only() {
    use crate::support::content::version;

    let platform = Platform::start_with_extensions(vec![shop()]).await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    platform.admin.content.add(
        "Sodium",
        version("sodium-1", "sodium", "1.21.4", "fabric", &[]),
    );
    let (created, _) = tokio::join!(
        platform.call(
            Method::POST,
            "/servers",
            Some(json!({
                "name": "Modé",
                "node_id": daemon.node.id.as_uuid(),
                "prefab_id": "minecraft-java",
                "owner_ref": ALICE,
                "settings": { "loader": "fabric", "version": "1.21.4" },
            })),
        ),
        daemon.accept(),
    );
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json);
    let id = created.json["id"].as_str().unwrap().to_owned();
    daemon.report(&id, "offline").await;
    platform.wait_for_state(&id, "offline").await;
    let mods = format!("/servers/{id}/content/mods");

    let slots = client(
        &platform,
        Method::GET,
        &format!("/servers/{id}/content"),
        Some("alice-token"),
        None,
    )
    .await;
    assert_eq!(slots.status, StatusCode::OK, "{}", slots.json);
    assert_eq!(slots.json[0]["slot"], "mods");
    let found = client(
        &platform,
        Method::GET,
        &format!("{mods}/search?q=sodium"),
        Some("alice-token"),
        None,
    )
    .await;
    assert_eq!(found.status, StatusCode::OK, "{}", found.json);
    assert_eq!(found.json["hits"][0]["title"], "Sodium");

    let (installed, request) = tokio::join!(
        client(
            &platform,
            Method::POST,
            &mods,
            Some("alice-token"),
            Some(json!({ "version_id": "sodium-1" }))
        ),
        daemon.accept_content(),
    );
    assert_eq!(installed.status, StatusCode::ACCEPTED, "{}", installed.json);
    daemon
        .event(
            "content.done",
            json!({ "server_id": id, "operation_id": request["payload"]["operation_id"], "status": "ok" }),
        )
        .await;
    let event = platform.admin.last_audit("server.content_installed").await;
    assert_eq!(
        event.details["owner_ref"], ALICE,
        "the owner is the audited actor"
    );

    // Le serveur d'un autre : introuvable, rien n'y est installé.
    let other = client(&platform, Method::GET, &mods, Some("bob-token"), None).await;
    assert_eq!(other.status, StatusCode::NOT_FOUND);
    let refused = client(
        &platform,
        Method::POST,
        &mods,
        Some("bob-token"),
        Some(json!({ "version_id": "sodium-1" })),
    )
    .await;
    assert_eq!(refused.status, StatusCode::NOT_FOUND);

    let listed = client(&platform, Method::GET, &mods, Some("alice-token"), None).await;
    let content_id = listed.json["items"][0]["id"].as_str().unwrap().to_owned();
    let item = format!("{mods}/{content_id}");
    let (removed, _) = tokio::join!(
        client(&platform, Method::DELETE, &item, Some("alice-token"), None),
        daemon.accept_content(),
    );
    assert_eq!(removed.status, StatusCode::OK, "{}", removed.json);
}
