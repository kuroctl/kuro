//! Notifications et webhooks (ADR-0044) : canaux (URL et secret jamais rendus, test signé),
//! règles et leur portée, émission depuis un crash, une sauvegarde en échec, le disque et la
//! présence d'un nœud, envoi avec reprise et abandon, limitation par canal, journal.

use axum::http::{Method, StatusCode};
use hmac::{Hmac, KeyInit, Mac};
use kuro_api::notifications::application as notifications;
use kuro_api::notifications::domain::{Event, EventKind, NodeRef};
use serde_json::{Value, json};
use sha2::Sha256;
use uuid::Uuid;

use crate::support::link::say_hello;
use crate::support::platform::Platform;

const BASE: &str = "/settings/notifications";

async fn create(platform: &Platform, path: &str, body: Value) -> Value {
    let response = platform
        .call(Method::POST, &format!("{BASE}/{path}"), Some(body))
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.json);
    response.json
}

fn webhook(name: &str, url: &str) -> Value {
    json!({ "name": name, "kind": "webhook", "url": url, "language": "en" })
}

/// Envois créés par l'émission détachée : attendus jusqu'à `expected`.
async fn wait_deliveries(platform: &Platform, expected: usize) -> Vec<Value> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let response = platform
            .call(Method::GET, &format!("{BASE}/deliveries"), None)
            .await;
        let deliveries = response.json.as_array().cloned().unwrap_or_default();
        if deliveries.len() >= expected {
            return deliveries;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "only {} deliveries: {deliveries:?}",
            deliveries.len()
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

fn header<'a>(
    outgoing: &'a kuro_api::notifications::sender::Outgoing,
    name: &str,
) -> Option<&'a str> {
    outgoing
        .headers
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
}

#[tokio::test]
async fn a_channel_hides_its_url_and_secret_and_its_test_is_signed() {
    let platform = Platform::start().await;
    let created = create(
        &platform,
        "channels",
        json!({
            "name": "Ops", "kind": "webhook", "url": "https://hooks.test/kuro/abcdef",
            "secret": "s3cret", "language": "en",
        }),
    )
    .await;
    assert_eq!(created["url_hint"], "https://hooks.test/kuro/abcdef");
    assert_eq!(created["has_secret"], true);
    assert!(created.get("url").is_none() && created.get("secret").is_none());
    let id = created["id"].as_str().unwrap().to_owned();
    let event = platform
        .admin
        .last_audit("notification.channel_created")
        .await;
    assert_eq!(event.details["url_hint"], "https://hooks.test/kuro/abcdef");
    assert!(!event.details.to_string().contains("s3cret"));

    // Test : un événement `test` signé part tout de suite.
    let test = platform
        .call(Method::POST, &format!("{BASE}/channels/{id}/test"), None)
        .await;
    assert_eq!(test.status, StatusCode::OK, "{}", test.json);
    assert_eq!(test.json["ok"], true);
    assert_eq!(test.json["status"], 204);
    let sent = platform.admin.webhooks.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].url, "https://hooks.test/kuro/abcdef");
    assert_eq!(header(&sent[0], "X-Kuro-Event"), Some("test"));
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(b"s3cret").unwrap();
    mac.update(&sent[0].body);
    let expected = format!(
        "sha256={}",
        mac.finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    assert_eq!(
        header(&sent[0], "X-Kuro-Signature"),
        Some(expected.as_str())
    );
    let body: Value = serde_json::from_slice(&sent[0].body).unwrap();
    assert_eq!(body["type"], "test");
    assert_eq!(body["title"], "Notification test");
    let tested = platform
        .admin
        .last_audit("notification.channel_tested")
        .await;
    assert_eq!(tested.outcome, "success");

    // Modification sans URL ni secret : gardés ; secret vide : retiré ; URL invalide : refusée.
    let renamed = platform
        .call(
            Method::PUT,
            &format!("{BASE}/channels/{id}"),
            Some(json!({ "name": "Ops 2", "kind": "webhook", "language": "fr" })),
        )
        .await;
    assert_eq!(renamed.status, StatusCode::OK, "{}", renamed.json);
    assert_eq!(renamed.json["has_secret"], true);
    assert_eq!(renamed.json["url_hint"], "https://hooks.test/kuro/abcdef");
    let cleared = platform
        .call(
            Method::PUT,
            &format!("{BASE}/channels/{id}"),
            Some(json!({ "name": "Ops 2", "kind": "webhook", "language": "fr", "secret": "" })),
        )
        .await;
    assert_eq!(cleared.json["has_secret"], false);
    let invalid = platform
        .call(
            Method::PUT,
            &format!("{BASE}/channels/{id}"),
            Some(
                json!({ "name": "Ops 2", "kind": "webhook", "language": "fr", "url": "http://x" }),
            ),
        )
        .await;
    assert_eq!(invalid.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(invalid.json["fields"][0]["field"], "url");
    assert_eq!(invalid.json["fields"][0]["code"], "invalid_webhook_url");

    let deleted = platform
        .call(Method::DELETE, &format!("{BASE}/channels/{id}"), None)
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    let list = platform
        .call(Method::GET, &format!("{BASE}/channels"), None)
        .await;
    assert_eq!(list.json, json!([]));
    let events = platform
        .call(Method::GET, &format!("{BASE}/events"), None)
        .await;
    assert!(
        events
            .json
            .as_array()
            .unwrap()
            .contains(&json!("server.crashed"))
    );
}

#[tokio::test]
async fn a_crash_is_delivered_to_the_rules_that_cover_the_server() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25566").await;
    let (server, _) = platform.deploy_minecraft(&mut daemon, json!({})).await;
    let id = server["id"].as_str().unwrap().to_owned();
    let hook = create(
        &platform,
        "channels",
        webhook("All", "https://hooks.test/all"),
    )
    .await;
    let discord = create(
        &platform,
        "channels",
        json!({ "name": "Discord", "kind": "discord", "url": "https://discord.com/api/webhooks/1/x", "language": "fr" }),
    )
    .await;
    let quiet = create(
        &platform,
        "channels",
        webhook("Quiet", "https://hooks.test/quiet"),
    )
    .await;
    let other_server = Uuid::now_v7();
    // Trois règles : tout, ce serveur, et un autre serveur (inexistant : refusée).
    create(
        &platform,
        "rules",
        json!({ "name": "Tout", "scope": "all", "events": ["server.crashed", "node.offline"], "channels": [hook["id"]] }),
    )
    .await;
    create(
        &platform,
        "rules",
        json!({ "name": "Survie", "scope": "server", "server_id": id, "events": ["server.crashed"], "channels": [discord["id"], hook["id"]] }),
    )
    .await;
    let refused = platform
        .call(
            Method::POST,
            &format!("{BASE}/rules"),
            Some(json!({ "name": "Autre", "scope": "server", "server_id": other_server, "events": ["server.crashed"], "channels": [quiet["id"]] })),
        )
        .await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(refused.json["fields"][0]["field"], "server_id");
    // Règle sur un autre événement : pas concernée.
    create(
        &platform,
        "rules",
        json!({ "name": "Nœuds", "scope": "all", "events": ["node.offline"], "channels": [quiet["id"]] }),
    )
    .await;
    let rule_event = platform.admin.last_audit("notification.rule_created").await;
    assert_eq!(rule_event.details["events"], json!(["node.offline"]));

    daemon.report(&id, "failed").await;
    platform.wait_for_state(&id, "failed").await;
    // Un message par canal, même visé par deux règles : hook une fois, discord une fois.
    let deliveries = wait_deliveries(&platform, 2).await;
    assert_eq!(deliveries.len(), 2, "{deliveries:?}");
    assert!(deliveries.iter().all(|d| d["status"] == "pending"));
    assert!(
        deliveries
            .iter()
            .all(|d| d["event"]["kind"] == "server.crashed")
    );

    let attempted = notifications::deliver_due(&platform.admin.notifications)
        .await
        .unwrap();
    assert_eq!(attempted, 2);
    let sent = platform.admin.webhooks.sent();
    let generic = sent
        .iter()
        .find(|o| o.url == "https://hooks.test/all")
        .unwrap();
    let body: Value = serde_json::from_slice(&generic.body).unwrap();
    assert_eq!(body["type"], "server.crashed");
    assert_eq!(body["server"]["name"], server["name"]);
    assert_eq!(
        body["server"]["url"],
        format!("https://panel.example.com/servers/{id}")
    );
    assert_eq!(header(generic, "X-Kuro-Signature"), None);
    let embed = sent
        .iter()
        .find(|o| o.url.starts_with("https://discord.com/"))
        .unwrap();
    let body: Value = serde_json::from_slice(&embed.body).unwrap();
    assert_eq!(
        body["embeds"][0]["title"],
        "Serveur arrêté de façon inattendue"
    );
    assert!(!sent.iter().any(|o| o.url == "https://hooks.test/quiet"));
    let after = platform
        .call(Method::GET, &format!("{BASE}/deliveries?limit=10"), None)
        .await;
    assert!(
        after
            .json
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["status"] == "sent" && d["attempts"] == 1)
    );
}

#[tokio::test]
async fn a_failed_backup_and_a_full_disk_are_notified() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25566").await;
    let (server, _) = platform.deploy_minecraft(&mut daemon, json!({})).await;
    let id = server["id"].as_str().unwrap().to_owned();
    daemon.report(&id, "running").await;
    platform.wait_for_state(&id, "running").await;
    let hook = create(
        &platform,
        "channels",
        webhook("Ops", "https://hooks.test/ops"),
    )
    .await;
    create(
        &platform,
        "rules",
        json!({ "name": "Tout", "scope": "all", "events": ["server.backup_failed", "server.disk_high"], "channels": [hook["id"]] }),
    )
    .await;

    // La création attend la réponse du nœud : les deux côtés avancent ensemble.
    let path = format!("/servers/{id}/backups");
    let (backup, request) = tokio::join!(
        platform.call(Method::POST, &path, Some(json!({}))),
        daemon.accept_backup(),
    );
    assert_eq!(backup.status, StatusCode::ACCEPTED, "{}", backup.json);
    daemon
        .event(
            "backup.done",
            json!({
                "server_id": id,
                "backup_id": request["payload"]["backup_id"],
                "status": "failed",
                "reason": "disk full",
            }),
        )
        .await;
    let deliveries = wait_deliveries(&platform, 1).await;
    assert_eq!(deliveries[0]["event"]["kind"], "server.backup_failed");
    assert_eq!(deliveries[0]["event"]["reason"], "disk full");

    // Disque : 91 % du quota une fois ; 95 % ensuite ne renotifie pas ; retour sous 80 % puis
    // 92 % notifie de nouveau.
    let quota = u64::try_from(server["disk_mb"].as_i64().unwrap()).unwrap() * 1024 * 1024;
    for (percent, expected) in [(91, 2), (95, 2), (50, 2), (92, 3)] {
        daemon
            .event(
                "server.disk",
                json!({ "server_id": id, "used_bytes": quota * percent / 100 }),
            )
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let deliveries = wait_deliveries(&platform, expected).await;
        assert_eq!(deliveries.len(), expected, "at {percent}%: {deliveries:?}");
    }
    let latest = wait_deliveries(&platform, 3).await;
    let disk = latest
        .iter()
        .find(|d| {
            d["event"]["kind"] == "server.disk_high" && d["event"]["details"]["percent"] == 92
        })
        .expect("disk event at 92%");
    assert_eq!(disk["event"]["server"]["name"], server["name"]);
}

#[tokio::test]
async fn node_presence_changes_are_notified_once_per_transition() {
    let platform = Platform::start().await;
    let node = platform.api.enroll_node("node-1").await;
    let hook = create(
        &platform,
        "channels",
        webhook("Ops", "https://hooks.test/ops"),
    )
    .await;
    create(
        &platform,
        "rules",
        json!({ "name": "Nœuds", "scope": "node", "node_id": node.id.as_uuid(), "events": ["node.offline", "node.online"], "channels": [hook["id"]] }),
    )
    .await;

    // Premier `hello` : en ligne.
    let mut ws = platform.api.connect(&node).await;
    say_hello(&mut ws).await;
    let deliveries = wait_deliveries(&platform, 1).await;
    assert_eq!(deliveries[0]["event"]["kind"], "node.online");
    assert_eq!(deliveries[0]["event"]["node"]["name"], "node-1");

    // Lien coupé : hors ligne ; de retour ensuite, une fois par transition.
    drop(ws);
    let deliveries = wait_deliveries(&platform, 2).await;
    assert_eq!(deliveries[0]["event"]["kind"], "node.offline");
    let mut ws = platform.api.connect(&node).await;
    say_hello(&mut ws).await;
    let deliveries = wait_deliveries(&platform, 3).await;
    assert_eq!(deliveries[0]["event"]["kind"], "node.online");

    // Un autre nœud n'est pas couvert par la règle.
    let other = platform.api.enroll_node("node-2").await;
    let mut other_ws = platform.api.connect(&other).await;
    say_hello(&mut other_ws).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let deliveries = wait_deliveries(&platform, 3).await;
    assert_eq!(deliveries.len(), 3, "{deliveries:?}");
}

#[tokio::test]
async fn failed_sends_are_retried_then_abandoned_and_busy_channels_wait() {
    let platform = Platform::start().await;
    let state = &platform.admin.notifications;
    let hook = create(
        &platform,
        "channels",
        webhook("Ops", "https://hooks.test/ops"),
    )
    .await;
    let disabled = create(
        &platform,
        "channels",
        json!({ "name": "Off", "kind": "webhook", "url": "https://hooks.test/off", "language": "en", "enabled": false }),
    )
    .await;
    create(
        &platform,
        "rules",
        json!({ "name": "Nœuds", "scope": "all", "events": ["node.offline"], "channels": [hook["id"], disabled["id"]] }),
    )
    .await;
    let event = || {
        Event::now(EventKind::NodeOffline).node(NodeRef {
            id: Uuid::now_v7(),
            name: "dev-1".to_owned(),
        })
    };
    // Canal désactivé : aucun envoi créé pour lui.
    assert_eq!(
        notifications::emit(&platform.admin.pool, event())
            .await
            .unwrap(),
        1
    );

    platform
        .admin
        .webhooks
        .fail("https://hooks.test/ops", "status_500");
    let force_due = || async {
        sqlx::query("UPDATE notification_deliveries SET next_attempt_at = now()")
            .execute(&platform.admin.pool)
            .await
            .unwrap();
    };
    for attempt in 1..=3 {
        assert_eq!(
            notifications::deliver_due(state).await.unwrap(),
            1,
            "attempt {attempt}"
        );
        let deliveries = wait_deliveries(&platform, 1).await;
        assert_eq!(deliveries[0]["status"], "pending", "attempt {attempt}");
        assert_eq!(deliveries[0]["attempts"], attempt);
        assert_eq!(deliveries[0]["last_error"], "status_500");
        // Pas encore due : rien ne part sans forcer l'échéance.
        assert_eq!(notifications::deliver_due(state).await.unwrap(), 0);
        force_due().await;
    }
    assert_eq!(notifications::deliver_due(state).await.unwrap(), 1);
    let deliveries = wait_deliveries(&platform, 1).await;
    assert_eq!(deliveries[0]["status"], "failed");
    assert_eq!(deliveries[0]["attempts"], 4);

    // Limitation : 30 envois par minute et par canal, le 31e attend.
    platform.admin.webhooks.succeed("https://hooks.test/ops");
    for _ in 0..31 {
        notifications::emit(&platform.admin.pool, event())
            .await
            .unwrap();
    }
    assert_eq!(notifications::deliver_due(state).await.unwrap(), 31);
    let deliveries = wait_deliveries(&platform, 32).await;
    let sent = deliveries.iter().filter(|d| d["status"] == "sent").count();
    let waiting = deliveries
        .iter()
        .filter(|d| d["status"] == "pending" && d["attempts"] == 0)
        .count();
    assert_eq!((sent, waiting), (30, 1), "{deliveries:?}");
}
