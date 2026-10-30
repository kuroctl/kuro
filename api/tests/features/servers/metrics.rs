//! Stats historisées (ADR-0043) : minutes reçues du nœud et relues sur 24 h, minutes d'un
//! autre nœud ignorées, cumul horaire et purge, plages 7 j et 90 j à l'heure.

use axum::http::{Method, StatusCode};
use kuro_api::servers::application::metrics;
use serde_json::{Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::support::platform::{FakeDaemon, Platform};

fn minute(server_id: &str, from: OffsetDateTime, cpu: u64, memory: u64) -> Value {
    let from_ms = from.unix_timestamp() * 1000;
    json!({
        "server_id": server_id,
        "from_ms": from_ms,
        "to_ms": from_ms + 60_000,
        "samples": 6,
        "cpu_millis_avg": cpu,
        "cpu_millis_max": cpu * 2,
        "memory_bytes_avg": memory,
        "memory_bytes_max": memory + 1024,
        "memory_limit_bytes": 2_147_483_648_u64,
        "network_rx_bytes": 1000,
        "network_tx_bytes": 500,
    })
}

/// Début de la minute courante.
fn now_minute() -> OffsetDateTime {
    let now = OffsetDateTime::now_utc();
    now.replace_second(0)
        .and_then(|t| t.replace_nanosecond(0))
        .unwrap_or(now)
}

async fn history(platform: &Platform, id: &str, range: &str) -> Value {
    let response = platform
        .call(
            Method::GET,
            &format!("/servers/{id}/metrics?range={range}"),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    response.json
}

async fn wait_points(platform: &Platform, id: &str, range: &str, expected: usize) -> Value {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let json = history(platform, id, range).await;
        if json["points"].as_array().map_or(0, Vec::len) >= expected {
            return json;
        }
        assert!(tokio::time::Instant::now() < deadline, "{json}");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

async fn server(platform: &Platform, daemon: &mut FakeDaemon) -> String {
    let (server, _) = platform.deploy_minecraft(daemon, json!({})).await;
    server["id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn minutes_sent_by_the_node_are_kept_and_read_over_a_day() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25566").await;
    let id = server(&platform, &mut daemon).await;
    let now = now_minute();
    daemon
        .event(
            "server.metrics",
            minute(&id, now - time::Duration::minutes(2), 250, 1_000_000),
        )
        .await;
    daemon
        .event(
            "server.metrics",
            minute(&id, now - time::Duration::minutes(1), 500, 2_000_000),
        )
        .await;
    let json = wait_points(&platform, &id, "24h", 2).await;
    assert_eq!(json["points"][1]["cpu_millis_avg"], 500);
    // Un renvoi de la même minute écrase.
    daemon
        .event(
            "server.metrics",
            minute(&id, now - time::Duration::minutes(1), 600, 2_000_000),
        )
        .await;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    let json = loop {
        let json = history(&platform, &id, "24h").await;
        if json["points"][1]["cpu_millis_avg"] == 600 {
            break json;
        }
        assert!(tokio::time::Instant::now() < deadline, "{json}");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    assert_eq!(json["range"], "24h");
    assert_eq!(json["step_secs"], 60);
    let points = json["points"].as_array().unwrap();
    assert_eq!(points.len(), 2, "{points:?}");
    assert_eq!(points[0]["cpu_millis_avg"], 250);
    assert_eq!(points[1]["cpu_millis_avg"], 600);
    assert_eq!(points[1]["cpu_millis_max"], 1200);
    assert_eq!(points[1]["memory_limit_bytes"], 2_147_483_648_u64);
    assert_eq!(
        points[1]["at"],
        (now - time::Duration::minutes(1)).format(&Rfc3339).unwrap()
    );
    // Sans plage : 24 h ; plage inconnue : refusée.
    let default = platform
        .call(Method::GET, &format!("/servers/{id}/metrics"), None)
        .await;
    assert_eq!(default.json["range"], "24h");
    let bad = platform
        .call(
            Method::GET,
            &format!("/servers/{id}/metrics?range=1y"),
            None,
        )
        .await;
    assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(bad.json["fields"][0]["field"], "range");
}

#[tokio::test]
async fn metrics_of_a_server_on_another_node_are_ignored() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25566").await;
    let id = server(&platform, &mut daemon).await;
    let mut other = platform.online_node("node-2", "25565-25566").await;
    other
        .event("server.metrics", minute(&id, now_minute(), 999, 1))
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let json = history(&platform, &id, "24h").await;
    assert_eq!(json["points"], json!([]));
}

#[tokio::test]
async fn old_minutes_are_rolled_up_into_hours_then_purged() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25566").await;
    let id = server(&platform, &mut daemon).await;
    let now = now_minute();
    // Trois minutes il y a deux jours, dans la même heure, et une minute il y a dix jours (à
    // purger).
    let old_hour = (now - time::Duration::days(2)).replace_minute(0).unwrap();
    for (offset, cpu) in [(0, 100), (1, 300), (2, 200)] {
        daemon
            .event(
                "server.metrics",
                minute(&id, old_hour + time::Duration::minutes(offset), cpu, 1_000),
            )
            .await;
    }
    daemon
        .event(
            "server.metrics",
            minute(&id, now - time::Duration::days(10), 50, 1_000),
        )
        .await;
    // Les quatre minutes sont enregistrées (tâches détachées) avant le cumul.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM server_metrics WHERE server_id = $1")
                .bind(uuid::Uuid::parse_str(&id).unwrap())
                .fetch_one(&platform.admin.pool)
                .await
                .unwrap();
        if count == 4 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{count} minutes recorded"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    let (hours, minutes, _) = metrics::maintain(&platform.admin.pool)
        .await
        .unwrap()
        .expect("lock taken");
    assert!(hours >= 2, "{hours} hours rolled up");
    assert_eq!(minutes, 1, "the ten-day-old minute is purged");

    let week = history(&platform, &id, "7d").await;
    assert_eq!(week["step_secs"], 3600);
    let points = week["points"].as_array().unwrap();
    let hour = points
        .iter()
        .find(|p| p["at"] == old_hour.format(&Rfc3339).unwrap())
        .expect("rolled up hour");
    assert_eq!(hour["samples"], 18);
    assert_eq!(hour["cpu_millis_avg"], 200);
    assert_eq!(hour["cpu_millis_max"], 600);
    assert_eq!(hour["network_rx_bytes"], 3000);
    // La minute de dix jours est purgée, mais son heure cumulée reste sur 90 jours.
    let quarter = history(&platform, &id, "90d").await;
    assert!(
        quarter["points"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["cpu_millis_avg"] == 50 && p["samples"] == 6),
        "{quarter}"
    );
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM server_metrics WHERE server_id = $1")
        .bind(uuid::Uuid::parse_str(&id).unwrap())
        .fetch_one(&platform.admin.pool)
        .await
        .unwrap();
    assert_eq!(left, 3);
    // Une seconde passe sans rien de nouveau ne casse rien.
    let again = metrics::maintain(&platform.admin.pool).await.unwrap();
    assert!(again.is_some());
}
