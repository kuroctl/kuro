//! Actions d'un prefab (ADR-0039) : commande rendue avec des paramètres validés, exécutée par le
//! nœud sur un serveur en marche, auditée sans sa sortie ; étape de tâche planifiée.

use axum::http::{Method, StatusCode};
use kuro_api::iam::domain::Permission;
use serde_json::{Value, json};

use crate::support::dns::TOKEN;
use crate::support::platform::{FakeDaemon, Platform};

/// Serveur WordPress en marche (zone DNS avec domaine des sites).
async fn running_wordpress() -> (Platform, FakeDaemon, String) {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let zone = platform
        .call(
            Method::PUT,
            "/settings/dns",
            Some(json!({
                "provider": "cloudflare", "base_domain": "play.kuroctl.test",
                "sites_domain": "web.kuroctl.test", "token": TOKEN
            })),
        )
        .await;
    assert_eq!(zone.status, StatusCode::OK, "{}", zone.json);
    let (response, _) = tokio::join!(
        platform.call(
            Method::POST,
            "/servers",
            Some(json!({
                "name": "Blog",
                "node_id": daemon.node.id.as_uuid(),
                "prefab_id": "wordpress",
                "memory_mb": 1024,
                "cpu_percent": 100,
                "disk_mb": 2048,
                "settings": {},
            })),
        ),
        daemon.accept(),
    );
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.json);
    let id = response.json["id"].as_str().unwrap().to_owned();
    daemon.report(&id, "running").await;
    platform.wait_for_state(&id, "running").await;
    (platform, daemon, id)
}

#[tokio::test]
async fn an_action_is_rendered_with_its_parameters_run_by_the_node_and_audited() {
    let (platform, mut daemon, id) = running_wordpress().await;
    let path = format!("/servers/{id}/actions/fix-owner");

    let (response, request) = tokio::join!(
        platform.call(
            Method::POST,
            &path,
            Some(json!({ "params": { "path": "/var/www/html/wp-content/uploads" } })),
        ),
        async {
            let request = daemon.next_command().await;
            daemon
                .answer(
                    &request,
                    "action.result",
                    json!({ "server_id": id, "exit_code": 0, "output": "done\n", "truncated": false }),
                )
                .await;
            request
        },
    );

    assert_eq!(request["type"], "server.action");
    assert_eq!(request["payload"]["action"], "fix-owner");
    assert_eq!(request["payload"]["service"], "web");
    assert_eq!(
        request["payload"]["command"],
        json!(["chown", "-R", "33:33", "/var/www/html/wp-content/uploads"])
    );
    assert_eq!(request["payload"]["timeout_secs"], 120);
    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(
        response.json,
        json!({ "exit_code": 0, "output": "done\n", "truncated": false })
    );
    let event = platform.admin.last_audit("server.action_ran").await;
    assert_eq!(event.details["action"], "fix-owner");
    assert_eq!(
        event.details["params"]["path"],
        "/var/www/html/wp-content/uploads"
    );
    assert_eq!(event.details["exit_code"], 0);
    assert!(
        event.details.get("output").is_none(),
        "the output is never audited"
    );

    // Paramètre par défaut quand il est absent.
    let (response, request) =
        tokio::join!(platform.call(Method::POST, &path, Some(json!({}))), async {
            let request = daemon.next_command().await;
            daemon
                .answer(
                    &request,
                    "action.result",
                    json!({ "server_id": id, "exit_code": 1, "output": "", "truncated": false }),
                )
                .await;
            request
        },);
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(request["payload"]["command"][3], "/var/www/html");
    assert_eq!(response.json["exit_code"], 1);
}

#[tokio::test]
async fn parameters_are_validated_before_anything_reaches_the_node() {
    let (platform, _daemon, id) = running_wordpress().await;

    let refused = platform
        .call(
            Method::POST,
            &format!("/servers/{id}/actions/fix-owner"),
            Some(json!({ "params": { "path": "/etc; rm -rf /" } })),
        )
        .await;
    assert_eq!(
        refused.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        refused.json
    );
    assert_eq!(refused.json["fields"][0]["field"], "params.path");
    assert_eq!(refused.json["fields"][0]["code"], "invalid_action_param");

    let unknown_param = platform
        .call(
            Method::POST,
            &format!("/servers/{id}/actions/fix-owner"),
            Some(json!({ "params": { "nope": "x" } })),
        )
        .await;
    assert_eq!(unknown_param.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(unknown_param.json["fields"][0]["field"], "params.nope");

    let unknown_action = platform
        .call(
            Method::POST,
            &format!("/servers/{id}/actions/nope"),
            Some(json!({})),
        )
        .await;
    assert_eq!(unknown_action.status, StatusCode::NOT_FOUND);
    assert_eq!(unknown_action.json["code"], "unknown_action");
    assert!(
        platform
            .admin
            .audit()
            .await
            .iter()
            .all(|e| e.action != "server.action_ran")
    );
}

#[tokio::test]
async fn a_stopped_server_refuses_actions() {
    let (platform, mut daemon, id) = running_wordpress().await;
    daemon.report(&id, "offline").await;
    platform.wait_for_state(&id, "offline").await;

    let refused = platform
        .call(
            Method::POST,
            &format!("/servers/{id}/actions/cron"),
            Some(json!({})),
        )
        .await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.json);
    assert_eq!(refused.json["code"], "server_not_running");
}

#[tokio::test]
async fn only_a_schedulable_action_becomes_a_step() {
    let (platform, _daemon, id) = running_wordpress().await;
    let schedule = |step: Value| {
        json!({
            "name": "Cron", "cron": "*/5 * * * *", "timezone": "UTC",
            "steps": [{ "action": "action", "value": step }]
        })
    };

    let accepted = platform
        .call(
            Method::POST,
            &format!("/servers/{id}/schedules"),
            Some(schedule(json!("cron"))),
        )
        .await;
    assert_eq!(accepted.status, StatusCode::CREATED, "{}", accepted.json);
    assert_eq!(accepted.json["steps"][0]["action"], "action");
    assert_eq!(accepted.json["steps"][0]["value"], "cron");

    // `fix-owner` n'est pas planifiable ; une action inconnue non plus.
    for value in ["fix-owner", "nope"] {
        let refused = platform
            .call(
                Method::POST,
                &format!("/servers/{id}/schedules"),
                Some(schedule(json!(value))),
            )
            .await;
        assert_eq!(
            refused.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{value}: {}",
            refused.json
        );
        assert_eq!(
            refused.json["fields"][0]["code"], "action_not_schedulable",
            "{value}"
        );
    }

    // L'étape exige le droit des actions, comme une commande exige celui de la console.
    let limited = platform
        .admin
        .account(
            "limited@example.com",
            &[Permission::ServersSchedulesManage, Permission::ServersRead],
        )
        .await;
    let session = platform.admin.login(&limited).await;
    let forbidden = platform
        .admin
        .call(
            Method::POST,
            &format!("/servers/{id}/schedules"),
            Some(&session.access_token),
            Some(schedule(json!("cron"))),
        )
        .await;
    assert_eq!(
        forbidden.status,
        StatusCode::FORBIDDEN,
        "{}",
        forbidden.json
    );
}
