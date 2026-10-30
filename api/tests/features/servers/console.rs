//! Console et stats en temps réel (ADR-0014) : ticket, origine, abonnement du nœud, relais au
//! navigateur, commandes auditées.

use std::net::SocketAddr;

use axum::http::{Method, StatusCode};
use kuro_api::iam::domain::Permission;
use serde_json::{Value, json};

use crate::support::browser::{Browser, Closed, serve};
use crate::support::http::PANEL_ORIGIN;
use crate::support::link::send;
use crate::support::platform::{FakeDaemon, Platform};

/// Plateforme, nœud et serveur Minecraft en marche, REST servi sur un vrai port.
struct Setup {
    platform: Platform,
    daemon: FakeDaemon,
    id: String,
    addr: SocketAddr,
}

async fn running_server() -> Setup {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565").await;
    let (server, _) = platform.deploy_minecraft(&mut daemon, json!({})).await;
    let id = server["id"].as_str().unwrap().to_owned();
    daemon.report(&id, "running").await;
    platform.wait_for_state(&id, "running").await;
    let addr = serve(platform.admin.router()).await;
    Setup {
        platform,
        daemon,
        id,
        addr,
    }
}

async fn ticket(platform: &Platform, id: &str) -> String {
    let response = platform
        .call(Method::POST, &format!("/servers/{id}/console/ticket"), None)
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.json);
    assert_eq!(response.json["expires_in_secs"], 30);
    response.json["ticket"].as_str().unwrap().to_owned()
}

/// Ouvre la console et attend l'abonnement du nœud.
async fn open_console(setup: &mut Setup) -> Browser {
    let ticket = ticket(&setup.platform, &setup.id).await;
    let mut browser = Browser::open(setup.addr, &setup.id, PANEL_ORIGIN)
        .await
        .unwrap();
    browser.auth(&ticket).await;
    let state = browser.next_of("state").await;
    assert_eq!(state["state"], "running");
    let watch = setup.daemon.accept().await;
    assert_eq!(watch["type"], "server.watch");
    assert_eq!(
        watch["payload"],
        json!({ "server_id": setup.id, "watch": true })
    );
    browser
}

async fn lines(daemon: &mut FakeDaemon, server_id: &str, lines: &[(i64, &str)]) {
    let lines: Vec<Value> = lines
        .iter()
        .map(|(at, text)| json!({ "at_ms": at, "text": text }))
        .collect();
    send(
        &mut daemon.ws,
        json!({
            "id": uuid::Uuid::now_v7(),
            "type": "console.line",
            "payload": { "server_id": server_id, "lines": lines },
        }),
    )
    .await;
}

#[tokio::test]
async fn a_console_relays_the_nodes_lines_and_stats() {
    let mut setup = running_server().await;
    let mut browser = open_console(&mut setup).await;

    lines(&mut setup.daemon, &setup.id, &[(1000, "Done (3.2s)!")]).await;
    send(
        &mut setup.daemon.ws,
        json!({
            "id": uuid::Uuid::now_v7(),
            "type": "server.stats",
            "payload": {
                "server_id": setup.id, "cpu_millis": 750, "memory_bytes": 1024,
                "memory_limit_bytes": 2048, "network_rx_bytes": 10, "network_tx_bytes": 20,
            },
        }),
    )
    .await;

    let received = browser.next_of("lines").await;
    assert_eq!(
        received["lines"],
        json!([{ "at_ms": 1000, "text": "Done (3.2s)!" }])
    );
    let stats = browser.next_of("stats").await;
    assert_eq!(stats["cpu_millis"], 750);
    assert_eq!(stats["memory_limit_bytes"], 2048);

    // État en direct, sans interroger le REST.
    setup.daemon.report(&setup.id, "stopping").await;
    assert_eq!(browser.next_of("state").await["state"], "stopping");

    let event = setup
        .platform
        .admin
        .last_audit("server.console_opened")
        .await;
    assert_eq!(event.target_id.as_deref(), Some(setup.id.as_str()));
}

#[tokio::test]
async fn the_node_stops_streaming_when_the_last_viewer_leaves() {
    let mut setup = running_server().await;
    let first = open_console(&mut setup).await;

    // Second spectateur : pas de nouvel abonnement, l'historique gardé par l'API lui suffit.
    lines(&mut setup.daemon, &setup.id, &[(1000, "earlier")]).await;
    let ticket = ticket(&setup.platform, &setup.id).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let mut second = Browser::open(setup.addr, &setup.id, PANEL_ORIGIN)
        .await
        .unwrap();
    second.auth(&ticket).await;
    let history = second.next_of("lines").await;
    assert_eq!(history["lines"][0]["text"], "earlier");

    first.close().await;
    second.close().await;

    let unwatch = setup.daemon.accept().await;
    assert_eq!(unwatch["type"], "server.watch");
    assert_eq!(unwatch["payload"]["watch"], false);
}

#[tokio::test]
async fn a_reconnecting_node_is_watched_again() {
    let mut setup = running_server().await;
    let _browser = open_console(&mut setup).await;

    let mut ws = setup.platform.api.connect(&setup.daemon.node).await;
    crate::support::link::say_hello(&mut ws).await;
    let watch = crate::support::link::recv(&mut ws).await.unwrap();

    assert_eq!(watch["type"], "server.watch");
    assert_eq!(
        watch["payload"],
        json!({ "server_id": setup.id, "watch": true })
    );
}

#[tokio::test]
async fn a_ticket_is_single_use_and_bound_to_its_server() {
    let setup = running_server().await;
    let ticket = ticket(&setup.platform, &setup.id).await;

    let mut first = Browser::open(setup.addr, &setup.id, PANEL_ORIGIN)
        .await
        .unwrap();
    first.auth(&ticket).await;
    first.next_of("state").await;

    let mut replay = Browser::open(setup.addr, &setup.id, PANEL_ORIGIN)
        .await
        .unwrap();
    replay.auth(&ticket).await;
    assert_eq!(replay.closed().await, Closed::Code(4401));

    // Ticket valide, mais pour un autre serveur.
    let other = uuid::Uuid::now_v7().to_string();
    let ticket = self::ticket(&setup.platform, &setup.id).await;
    let mut elsewhere = Browser::open(setup.addr, &other, PANEL_ORIGIN)
        .await
        .unwrap();
    elsewhere.auth(&ticket).await;
    assert_eq!(elsewhere.closed().await, Closed::Code(4401));

    let denied = setup.platform.admin.last_audit("access.denied").await;
    assert_eq!(denied.details["reason"], "console_ticket");
}

#[tokio::test]
async fn a_console_without_ticket_is_closed() {
    let setup = running_server().await;

    let mut garbage = Browser::open(setup.addr, &setup.id, PANEL_ORIGIN)
        .await
        .unwrap();
    garbage.auth("kct_0000").await;
    assert_eq!(garbage.closed().await, Closed::Code(4401));

    let mut wrong_message = Browser::open(setup.addr, &setup.id, PANEL_ORIGIN)
        .await
        .unwrap();
    wrong_message
        .send(json!({ "type": "command", "line": "stop" }))
        .await;
    assert_eq!(wrong_message.closed().await, Closed::Code(4401));
}

#[tokio::test]
async fn a_foreign_origin_cannot_open_a_console() {
    let setup = running_server().await;

    let refused = Browser::open(setup.addr, &setup.id, "https://evil.example.com").await;

    assert_eq!(refused.err(), Some(403));
    let denied = setup.platform.admin.last_audit("access.denied").await;
    assert_eq!(denied.details["reason"], "origin");
    assert_eq!(denied.details["origin"], "https://evil.example.com");
}

#[tokio::test]
async fn the_apis_own_origin_is_accepted() {
    let setup = running_server().await;
    let ticket = ticket(&setup.platform, &setup.id).await;

    let mut browser = Browser::open(setup.addr, &setup.id, &format!("http://{}", setup.addr))
        .await
        .unwrap();
    browser.auth(&ticket).await;

    assert_eq!(browser.next_of("state").await["state"], "running");
}

#[tokio::test]
async fn lines_from_another_node_are_ignored() {
    let mut setup = running_server().await;
    let mut intruder = setup.platform.online_node("node-2", "25566").await;
    let mut browser = open_console(&mut setup).await;

    lines(&mut intruder, &setup.id, &[(1000, "forged")]).await;
    // Le nœud légitime ensuite : si la ligne de l'intrus était relayée, elle arriverait avant.
    lines(&mut setup.daemon, &setup.id, &[(2000, "genuine")]).await;

    let received = browser.next_of("lines").await;
    assert_eq!(
        received["lines"],
        json!([{ "at_ms": 2000, "text": "genuine" }])
    );
    // Et elle n'arrive pas après non plus.
    lines(&mut setup.daemon, &setup.id, &[(3000, "later")]).await;
    let received = browser.next_of("lines").await;
    assert_eq!(
        received["lines"],
        json!([{ "at_ms": 3000, "text": "later" }])
    );
}

#[tokio::test]
async fn a_command_is_forwarded_and_audited() {
    let mut setup = running_server().await;
    let path = format!("/servers/{}/command", setup.id);

    let (accepted, command) = tokio::join!(
        setup
            .platform
            .call(Method::POST, &path, Some(json!({ "line": "op alice" }))),
        setup.daemon.accept(),
    );

    assert_eq!(accepted.status, StatusCode::ACCEPTED, "{}", accepted.json);
    assert_eq!(command["type"], "server.command");
    assert_eq!(
        command["payload"],
        json!({ "server_id": setup.id, "line": "op alice" })
    );
    let event = setup.platform.admin.last_audit("server.command").await;
    assert_eq!(event.details["line"], "op alice");
}

#[tokio::test]
async fn an_audited_command_is_truncated() {
    let mut setup = running_server().await;
    let line = format!("say {}", "a".repeat(600));
    let path = format!("/servers/{}/command", setup.id);

    let (accepted, _) = tokio::join!(
        setup
            .platform
            .call(Method::POST, &path, Some(json!({ "line": line }))),
        setup.daemon.accept(),
    );

    assert_eq!(accepted.status, StatusCode::ACCEPTED);
    let event = setup.platform.admin.last_audit("server.command").await;
    assert_eq!(event.details["line"].as_str().unwrap().chars().count(), 256);
}

#[tokio::test]
async fn an_injected_or_empty_command_is_refused() {
    let setup = running_server().await;
    let path = format!("/servers/{}/command", setup.id);

    for line in ["stop\nop attacker", "", "say \u{1b}[2J"] {
        let response = setup
            .platform
            .call(Method::POST, &path, Some(json!({ "line": line })))
            .await;
        assert_eq!(
            response.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{line:?}"
        );
        assert_eq!(response.json["fields"][0]["field"], "line");
        assert_eq!(response.json["fields"][0]["code"], "invalid_command");
    }
}

#[tokio::test]
async fn a_command_needs_a_running_server() {
    let mut setup = running_server().await;
    let path = format!("/servers/{}/command", setup.id);

    // Le nœud constate que le serveur s'est arrêté.
    let (refused, ()) = tokio::join!(
        setup
            .platform
            .call(Method::POST, &path, Some(json!({ "line": "list" }))),
        async {
            let command = setup.daemon.next_command().await;
            setup
                .daemon
                .reply(
                    &command,
                    json!({ "status": "error", "code": "not_running", "message": "x" }),
                )
                .await;
        },
    );
    assert_eq!(refused.status, StatusCode::CONFLICT);
    assert_eq!(refused.json["code"], "server_not_running");

    // Arrêté selon l'API : refusé sans solliciter le nœud.
    setup.daemon.report(&setup.id, "offline").await;
    setup.platform.wait_for_state(&setup.id, "offline").await;
    let refused = setup
        .platform
        .call(Method::POST, &path, Some(json!({ "line": "list" })))
        .await;
    assert_eq!(refused.status, StatusCode::CONFLICT);
    assert_eq!(refused.json["code"], "server_not_running");
}

#[tokio::test]
async fn console_access_requires_its_permissions() {
    let setup = running_server().await;
    let account = setup
        .platform
        .admin
        .account("alice@example.com", &[Permission::ServersRead])
        .await;
    let token = setup.platform.admin.login(&account).await.access_token;

    let ticket = setup
        .platform
        .admin
        .call(
            Method::POST,
            &format!("/servers/{}/console/ticket", setup.id),
            Some(&token),
            None,
        )
        .await;
    let command = setup
        .platform
        .admin
        .call(
            Method::POST,
            &format!("/servers/{}/command", setup.id),
            Some(&token),
            Some(json!({ "line": "op alice" })),
        )
        .await;

    assert_eq!(ticket.status, StatusCode::FORBIDDEN);
    assert_eq!(ticket.json["params"]["permission"], "servers.console.read");
    assert_eq!(command.status, StatusCode::FORBIDDEN);
    assert_eq!(
        command.json["params"]["permission"],
        "servers.console.write"
    );
}
