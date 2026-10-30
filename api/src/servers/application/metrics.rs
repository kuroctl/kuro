//! Stats historisées (ADR-0043) : enregistrement des minutes envoyées par les nœuds, lecture
//! par plage (24 h à la minute, 7 j et 90 j à l'heure), cumul horaire et purge périodiques
//! sous verrou consultatif (une instance à la fois, ADR-0037).

use std::time::Duration;

use kuro_protocol::daemon::ServerMetrics;
use sqlx::PgPool;
use time::OffsetDateTime;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{ServersError, ServersState};
use crate::nodes::domain::NodeId;
use crate::servers::infra::{MetricPoint, MetricsRepository, ServerRepository};

/// Minutes gardées sept jours, heures quatre-vingt-dix.
const KEEP_MINUTES: time::Duration = time::Duration::days(7);
const KEEP_HOURS: time::Duration = time::Duration::days(90);
/// Cumul et purge toutes les heures ; les minutes de l'heure en cours restent telles quelles.
const MAINTAIN_EVERY: Duration = Duration::from_secs(3600);
/// Clé du verrou consultatif du cumul.
const ROLLUP_LOCK: i64 = 0x6b75_726f_5f6d_6574; // "kuro_met"

/// Plage demandée par le panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    /// 24 heures, à la minute.
    Day,
    /// 7 jours, à l'heure.
    Week,
    /// 90 jours, à l'heure.
    Quarter,
}

impl Range {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "24h" => Some(Self::Day),
            "7d" => Some(Self::Week),
            "90d" => Some(Self::Quarter),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Day => "24h",
            Self::Week => "7d",
            Self::Quarter => "90d",
        }
    }

    /// Pas entre deux points, en secondes.
    pub fn step_secs(self) -> u32 {
        match self {
            Self::Day => 60,
            Self::Week | Self::Quarter => 3600,
        }
    }

    fn span(self) -> time::Duration {
        match self {
            Self::Day => time::Duration::hours(24),
            Self::Week => time::Duration::days(7),
            Self::Quarter => time::Duration::days(90),
        }
    }
}

/// Une minute reçue d'un nœud : enregistrée si le serveur est à ce nœud, ignorée sinon.
pub async fn record(
    pool: &PgPool,
    node: NodeId,
    metrics: ServerMetrics,
) -> Result<(), sqlx::Error> {
    let Ok(at) = OffsetDateTime::from_unix_timestamp(metrics.from_ms.div_euclid(1000)) else {
        tracing::warn!(server_id = %metrics.server_id, "ignored metrics with an invalid time");
        return Ok(());
    };
    let clamp = |value: u64| i64::try_from(value).unwrap_or(i64::MAX);
    let point = MetricPoint {
        at,
        samples: i32::try_from(metrics.samples).unwrap_or(i32::MAX),
        cpu_millis_avg: clamp(metrics.cpu_millis_avg),
        cpu_millis_max: clamp(metrics.cpu_millis_max),
        memory_bytes_avg: clamp(metrics.memory_bytes_avg),
        memory_bytes_max: clamp(metrics.memory_bytes_max),
        memory_limit_bytes: clamp(metrics.memory_limit_bytes),
        network_rx_bytes: clamp(metrics.network_rx_bytes),
        network_tx_bytes: clamp(metrics.network_tx_bytes),
    };
    let mut conn = pool.acquire().await?;
    if !MetricsRepository::upsert_if_owned(&mut conn, node.as_uuid(), metrics.server_id, &point)
        .await?
    {
        tracing::warn!(%node, server_id = %metrics.server_id, "ignored metrics of a server not on this node");
    }
    Ok(())
}

/// Historique d'un serveur sur `range`, prêt à tracer.
pub async fn history(
    state: &ServersState,
    server_id: Uuid,
    range: Range,
) -> Result<Vec<MetricPoint>, ServersError> {
    let mut conn = state.pool.acquire().await?;
    ServerRepository::get(&mut conn, server_id)
        .await?
        .ok_or(ServersError::NotFound)?;
    let since = OffsetDateTime::now_utc() - range.span();
    Ok(match range {
        Range::Day => MetricsRepository::minutes(&mut conn, server_id, since).await?,
        Range::Week | Range::Quarter => {
            // Les heures cumulées, puis les minutes pas encore cumulées de l'heure en cours.
            let mut points = MetricsRepository::hours(&mut conn, server_id, since).await?;
            let last_hour = points.last().map(|p| p.at + time::Duration::hours(1));
            let recent_since = last_hour.unwrap_or(since).max(since);
            let recent = MetricsRepository::minutes(&mut conn, server_id, recent_since).await?;
            if let Some(partial) = fold_hour(&recent) {
                points.push(partial);
            }
            points
        }
    })
}

/// Cumul d'une heure entamée depuis ses minutes, pour terminer une courbe à l'heure.
fn fold_hour(minutes: &[MetricPoint]) -> Option<MetricPoint> {
    let first = minutes.first()?;
    let samples: i64 = minutes
        .iter()
        .map(|m| i64::from(m.samples))
        .sum::<i64>()
        .max(1);
    let weighted = |field: fn(&MetricPoint) -> i64| {
        minutes
            .iter()
            .map(|m| field(m) * i64::from(m.samples))
            .sum::<i64>()
            / samples
    };
    Some(MetricPoint {
        at: first.at.replace_minute(0).ok()?.replace_second(0).ok()?,
        samples: i32::try_from(samples).unwrap_or(i32::MAX),
        cpu_millis_avg: weighted(|m| m.cpu_millis_avg),
        cpu_millis_max: minutes.iter().map(|m| m.cpu_millis_max).max().unwrap_or(0),
        memory_bytes_avg: weighted(|m| m.memory_bytes_avg),
        memory_bytes_max: minutes
            .iter()
            .map(|m| m.memory_bytes_max)
            .max()
            .unwrap_or(0),
        memory_limit_bytes: minutes
            .iter()
            .map(|m| m.memory_limit_bytes)
            .max()
            .unwrap_or(0),
        network_rx_bytes: minutes.iter().map(|m| m.network_rx_bytes).sum(),
        network_tx_bytes: minutes.iter().map(|m| m.network_tx_bytes).sum(),
    })
}

/// Cumul des heures révolues et purge, sous verrou consultatif ; rend (heures écrites, minutes
/// purgées, heures purgées) ou `None` si une autre instance s'en charge.
pub async fn maintain(pool: &PgPool) -> Result<Option<(u64, u64, u64)>, sqlx::Error> {
    let mut lock = pool.acquire().await?;
    let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
        .bind(ROLLUP_LOCK)
        .fetch_one(&mut *lock)
        .await?;
    if !locked {
        return Ok(None);
    }
    let result = maintain_locked(pool).await;
    if let Err(error) = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(ROLLUP_LOCK)
        .execute(&mut *lock)
        .await
    {
        tracing::warn!(%error, "failed to release the metrics rollup lock");
    }
    result.map(Some)
}

async fn maintain_locked(pool: &PgPool) -> Result<(u64, u64, u64), sqlx::Error> {
    let now = OffsetDateTime::now_utc();
    let current_hour = now
        .replace_minute(0)
        .and_then(|t| t.replace_second(0))
        .and_then(|t| t.replace_nanosecond(0))
        .unwrap_or(now);
    let mut conn = pool.acquire().await?;
    let hours = MetricsRepository::rollup(&mut conn, current_hour).await?;
    let (minutes, old_hours) =
        MetricsRepository::purge(&mut conn, now - KEEP_MINUTES, now - KEEP_HOURS).await?;
    Ok((hours, minutes, old_hours))
}

/// Boucle de l'API : cumul et purge toutes les heures.
pub async fn run_forever(pool: PgPool, shutdown: CancellationToken) {
    let mut ticks = tokio::time::interval(MAINTAIN_EVERY);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = shutdown.cancelled() => break,
            _ = ticks.tick() => {}
        }
        match maintain(&pool).await {
            Ok(Some((hours, minutes, old))) if hours + minutes + old > 0 => {
                tracing::info!(hours, minutes, old, "server metrics rolled up and purged");
            }
            Ok(_) => {}
            Err(error) => tracing::error!(%error, "server metrics maintenance failed"),
        }
    }
}
