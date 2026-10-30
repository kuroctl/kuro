//! Seuils d'alerte des stats (ADR-0043, amendement) : CPU soutenu pendant la durée réglée dans
//! les paramètres → `server.cpu_high` une fois, silence tant que ça reste haut, réarmement une
//! fois sous le seuil bas ; mémoire de même, rapportée à la limite du conteneur.

use axum::http::{Method, StatusCode};
use kuro_api::notifications::application as notifications;
use serde_json::{Value, json};
use time::OffsetDateTime;

use crate::support::platform::{FakeDaemon, Platform};

const MIB: u64 = 1024 * 1024;

fn minute(server_id: &str, from: OffsetDateTime, cpu_millis: u64, memory_bytes: u64) -> Value {
    let from_ms = from.unix_timestamp() * 1000;
    json!({
        "server_id": server_id,
        "from_ms": from_ms,
        "to_ms": from_ms + 60_000,
        "samples": 6,
        "cpu_millis_avg": cpu_millis,
        "cpu_millis_max": cpu_millis,
        "memory_bytes_avg": memory_bytes,
        "memory_bytes_max": memory_bytes,
        "memory_limit_bytes": 2048 * MIB,
        "network_rx_bytes": 0,
        "network_tx_bytes": 0,
    })
}

/// Début de la minute courante, moins un quart d'heure : les minutes du test restent passées.
fn base_minute() -> OffsetDateTime {
    let now = OffsetDateTime::now_utc() - time::Duration::minutes(15);
    now.replace_second(0)
        .and_then(|t| t.replace_nanosecond(0))
        .unwrap_or(now)
}

struct Setup {
    platform: Platform,
    daemon: FakeDaemon,
    id: String,
    base: OffsetDateTime,
}

impl Setup {
    /// Envoie la minute `offset` (en minutes après la base) et attend qu'elle soit enregistrée.
    async fn send(&mut self, offset: i64, cpu_millis: u64, memory_bytes: u64) {
        let at = self.base + time::Duration::minutes(offset);
        self.daemon
            .event(
                "server.metrics",
                minute(&self.id, at, cpu_millis, memory_bytes),
            )
            .await;
        let wanted = at
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let response = self
                .platform
                .call(
                    Method::GET,
                    &format!("/servers/{}/metrics?range=24h", self.id),
                    None,
                )
                .await;
            let points = response.json["points"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if points.iter().any(|p| p["at"] == wanted) {
                // L'évaluation suit l'enregistrement dans la même tâche : un court délai suffit.
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                return;
            }
            assert!(tokio::time::Instant::now() < deadline, "{points:?}");
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    async fn deliveries(&self) -> Vec<Value> {
        let response = self
            .platform
            .call(
                Method::GET,
                &format!("/servers/{}/notifications/deliveries", self.id),
                None,
            )
            .await;
        response.json.as_array().cloned().unwrap_or_default()
    }

    async fn wait_deliveries(&self, expected: usize) -> Vec<Value> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let deliveries = self.deliveries().await;
            if deliveries.len() >= expected {
                return deliveries;
            }
            assert!(tokio::time::Instant::now() < deadline, "{deliveries:?}");
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
}

async fn setup() -> Setup {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25566").await;
    let (server, _) = platform.deploy_minecraft(&mut daemon, json!({})).await;
    let id = server["id"].as_str().unwrap().to_owned();
    // Deux minutes suffisent pour le test ; seuils par défaut (90 %, réarmés sous 70 %).
    let settings = platform
        .call(
            Method::PATCH,
            "/settings",
            Some(json!({ "alert_minutes": 2 })),
        )
        .await;
    assert_eq!(settings.status, StatusCode::OK, "{}", settings.json);
    assert_eq!(settings.json["alert_cpu_percent"], 90);
    assert_eq!(settings.json["alert_memory_percent"], 90);
    let subscribed = platform
        .call(
            Method::POST,
            &format!("/servers/{id}/notifications"),
            Some(json!({
                "name": "Alertes", "events": ["server.cpu_high", "server.memory_high"],
                "kind": "webhook", "url": "https://hooks.test/alerts", "language": "fr",
            })),
        )
        .await;
    assert_eq!(
        subscribed.status,
        StatusCode::CREATED,
        "{}",
        subscribed.json
    );
    Setup {
        platform,
        daemon,
        id,
        base: base_minute(),
    }
}

#[tokio::test]
async fn sustained_cpu_raises_one_alert_then_rearms_under_the_low_threshold() {
    let mut setup = setup().await;
    // Le serveur a 200 % de CPU : 1 900 millicœurs font 95 %.
    setup.send(0, 1900, 500 * MIB).await;
    assert_eq!(
        setup.deliveries().await.len(),
        0,
        "one high minute is not enough"
    );
    setup.send(1, 1950, 500 * MIB).await;
    let deliveries = setup.wait_deliveries(1).await;
    assert_eq!(deliveries[0]["event"]["kind"], "server.cpu_high");
    assert_eq!(deliveries[0]["event"]["details"]["percent"], 98);
    assert_eq!(deliveries[0]["event"]["details"]["threshold_percent"], 90);
    assert_eq!(deliveries[0]["event"]["details"]["minutes"], 2);

    // Toujours haut, puis entre les deux seuils : rien de plus.
    setup.send(2, 1990, 500 * MIB).await;
    setup.send(3, 1500, 500 * MIB).await;
    assert_eq!(setup.deliveries().await.len(), 1, "still high: no repeat");

    // Sous 70 % : réarmée ; deux minutes hautes de nouveau : seconde alerte.
    setup.send(4, 1000, 500 * MIB).await;
    setup.send(5, 1900, 500 * MIB).await;
    setup.send(6, 1900, 500 * MIB).await;
    let deliveries = setup.wait_deliveries(2).await;
    assert!(
        deliveries
            .iter()
            .all(|d| d["event"]["kind"] == "server.cpu_high"),
        "{deliveries:?}"
    );
    // Le message est rendu en français, avec le pourcentage et la durée. La boucle d'envoi
    // de fond peut déjà tenir l'un des deux envois : on les attend tous les deux.
    notifications::deliver_due(&setup.platform.admin.notifications)
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    let sent = loop {
        let sent = setup.platform.admin.webhooks.sent();
        if sent.len() >= 2 {
            break sent;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{} sent",
            sent.len()
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    let bodies: Vec<Value> = sent
        .iter()
        .map(|o| serde_json::from_slice(&o.body).unwrap())
        .collect();
    assert!(
        bodies.iter().all(|b| b["type"] == "server.cpu_high"),
        "{bodies:?}"
    );
    assert!(
        bodies.iter().any(|b| b["message"]
            .as_str()
            .unwrap()
            .contains("95 % de son processeur depuis 2 min")),
        "{bodies:?}"
    );
}

#[tokio::test]
async fn sustained_memory_raises_an_alert_against_the_container_limit() {
    let mut setup = setup().await;
    // Limite de 2 Gio dans la minute : 1 900 Mio font 93 %.
    setup.send(0, 100, 1900 * MIB).await;
    setup.send(1, 100, 1900 * MIB).await;
    let deliveries = setup.wait_deliveries(1).await;
    assert_eq!(deliveries[0]["event"]["kind"], "server.memory_high");
    assert_eq!(deliveries[0]["event"]["details"]["percent"], 93);
    // La mémoire qui redescend ne réveille pas le CPU, resté bas.
    setup.send(2, 100, 500 * MIB).await;
    setup.send(3, 100, 500 * MIB).await;
    assert_eq!(setup.deliveries().await.len(), 1);
}
