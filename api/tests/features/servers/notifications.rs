//! Abonnements d'un serveur aux notifications (ADR-0044, amendement) : destination propre
//! (canal possédé, invisible des paramètres) ou canal partagé (avec le droit de le voir),
//! événements du serveur seulement, envoi après un crash, journal du serveur, suppression.

use axum::http::{Method, StatusCode};
use kuro_api::iam::domain::Permission;
use kuro_api::notifications::application as notifications;
use serde_json::{Value, json};

use crate::support::platform::{FakeDaemon, Platform};

struct Setup {
    platform: Platform,
    daemon: FakeDaemon,
    id: String,
}

async fn setup() -> Setup {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25566").await;
    let (server, _) = platform.deploy_minecraft(&mut daemon, json!({})).await;
    let id = server["id"].as_str().unwrap().to_owned();
    Setup {
        platform,
        daemon,
        id,
    }
}

async fn wait_deliveries(setup: &Setup, expected: usize) -> Vec<Value> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let response = setup
            .platform
            .call(
                Method::GET,
                &format!("/servers/{}/notifications/deliveries", setup.id),
                None,
            )
            .await;
        let deliveries = response.json.as_array().cloned().unwrap_or_default();
        if deliveries.len() >= expected {
            return deliveries;
        }
        assert!(tokio::time::Instant::now() < deadline, "{deliveries:?}");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn a_server_subscribes_with_its_own_destination_and_receives_its_crashes() {
    let mut setup = setup().await;
    let path = format!("/servers/{}/notifications", setup.id);
    let listed = setup.platform.call(Method::GET, &path, None).await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.json);
    assert_eq!(listed.json["subscriptions"], json!([]));
    assert_eq!(listed.json["shared_allowed"], true);
    assert!(
        listed.json["events"]
            .as_array()
            .unwrap()
            .contains(&json!("server.crashed"))
    );
    assert!(
        !listed.json["events"]
            .as_array()
            .unwrap()
            .contains(&json!("node.offline"))
    );

    let created = setup
        .platform
        .call(
            Method::POST,
            &path,
            Some(json!({
                "name": "Mon Discord", "events": ["server.crashed", "server.backup_failed"],
                "kind": "discord", "url": "https://discord.com/api/webhooks/42/token", "language": "fr",
            })),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json);
    assert_eq!(created.json["owned"], true);
    assert_eq!(created.json["channel"]["kind"], "discord");
    assert_eq!(
        created.json["channel"]["url_hint"],
        "https://discord.com/api/webhook…"
    );
    assert!(created.json["channel"].get("url").is_none());
    let sub = created.json["id"].as_str().unwrap().to_owned();
    // Le canal propre n'apparaît pas dans les paramètres, ni sa règle.
    let global = setup
        .platform
        .call(Method::GET, "/settings/notifications/channels", None)
        .await;
    assert_eq!(global.json, json!([]));
    let rules = setup
        .platform
        .call(Method::GET, "/settings/notifications/rules", None)
        .await;
    assert_eq!(rules.json, json!([]));

    // Événements de nœud refusés pour un serveur.
    let refused = setup
        .platform
        .call(
            Method::POST,
            &path,
            Some(json!({ "name": "Nœud", "events": ["node.offline"], "kind": "webhook", "url": "https://hooks.test/x" })),
        )
        .await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(refused.json["fields"][0]["field"], "events");

    // Crash : un envoi pour ce serveur, visible dans son journal, livré sur Discord.
    setup.daemon.report(&setup.id, "failed").await;
    setup.platform.wait_for_state(&setup.id, "failed").await;
    let deliveries = wait_deliveries(&setup, 1).await;
    assert_eq!(deliveries[0]["event"]["kind"], "server.crashed");
    notifications::deliver_due(&setup.platform.admin.notifications)
        .await
        .unwrap();
    let sent = setup.platform.admin.webhooks.sent();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].url.starts_with("https://discord.com/"));

    // Test et modification (URL gardée), puis suppression : le canal propre part avec.
    let tested = setup
        .platform
        .call(Method::POST, &format!("{path}/{sub}/test"), None)
        .await;
    assert_eq!(tested.json["ok"], true);
    let updated = setup
        .platform
        .call(
            Method::PUT,
            &format!("{path}/{sub}"),
            Some(json!({ "name": "Mon Discord", "events": ["server.crashed"], "kind": "discord", "language": "en" })),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.json);
    assert_eq!(updated.json["events"], json!(["server.crashed"]));
    assert_eq!(updated.json["channel"]["language"], "en");
    assert_eq!(
        updated.json["channel"]["url_hint"],
        "https://discord.com/api/webhook…"
    );
    let channel_id = updated.json["channel"]["id"].as_str().unwrap().to_owned();
    let deleted = setup
        .platform
        .call(Method::DELETE, &format!("{path}/{sub}"), None)
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    let gone = setup.platform.call(Method::GET, &path, None).await;
    assert_eq!(gone.json["subscriptions"], json!([]));
    let orphan = setup
        .platform
        .call(
            Method::POST,
            &format!("/settings/notifications/channels/{channel_id}/test"),
            None,
        )
        .await;
    assert_eq!(orphan.status, StatusCode::NOT_FOUND);
    let event = setup
        .platform
        .admin
        .last_audit("notification.rule_deleted")
        .await;
    assert_eq!(event.target_id.as_deref(), Some(setup.id.as_str()));
}

#[tokio::test]
async fn a_shared_channel_can_be_used_only_by_who_may_see_it() {
    let setup = setup().await;
    let path = format!("/servers/{}/notifications", setup.id);
    let shared = setup
        .platform
        .call(
            Method::POST,
            "/settings/notifications/channels",
            Some(json!({ "name": "Équipe", "kind": "slack", "url": "https://hooks.slack.com/services/x", "language": "fr" })),
        )
        .await;
    assert_eq!(shared.status, StatusCode::CREATED);
    let shared_id = shared.json["id"].as_str().unwrap().to_owned();

    let created = setup
        .platform
        .call(
            Method::POST,
            &path,
            Some(json!({ "name": "Vers l'équipe", "events": ["server.crashed"], "channel_id": shared_id })),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json);
    assert_eq!(created.json["owned"], false);
    assert_eq!(created.json["channel"]["name"], "Équipe");
    // Le canal partagé reste dans les paramètres ; la règle du serveur n'y figure pas.
    let channels = setup
        .platform
        .call(Method::GET, "/settings/notifications/channels", None)
        .await;
    assert_eq!(channels.json.as_array().unwrap().len(), 1);

    // Un admin qui ne voit pas les paramètres ne peut pas viser un canal partagé, mais peut
    // créer sa propre destination.
    let admin = &setup.platform.admin;
    let limited = admin
        .account(
            "ops@example.com",
            &[
                Permission::ServersRead,
                Permission::ServersNotificationsRead,
                Permission::ServersNotificationsManage,
            ],
        )
        .await;
    let token = admin.login(&limited).await.access_token;
    let listed = admin.call(Method::GET, &path, Some(&token), None).await;
    assert_eq!(listed.json["shared_allowed"], false);
    let refused = admin
        .call(
            Method::POST,
            &path,
            Some(&token),
            Some(json!({ "name": "Vers l'équipe", "events": ["server.crashed"], "channel_id": shared_id })),
        )
        .await;
    assert_eq!(
        refused.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        refused.json
    );
    assert_eq!(refused.json["fields"][0]["field"], "channel_id");
    let own = admin
        .call(
            Method::POST,
            &path,
            Some(&token),
            Some(json!({ "name": "Mon webhook", "events": ["server.disk_high"], "kind": "webhook", "url": "https://hooks.test/mine", "secret": "s" })),
        )
        .await;
    assert_eq!(own.status, StatusCode::CREATED, "{}", own.json);
    assert_eq!(own.json["channel"]["has_secret"], true);
    // Sans le droit des paramètres, pas d'accès à la page globale.
    let global = admin
        .call(
            Method::GET,
            "/settings/notifications/channels",
            Some(&token),
            None,
        )
        .await;
    assert_eq!(global.status, StatusCode::FORBIDDEN);
}
