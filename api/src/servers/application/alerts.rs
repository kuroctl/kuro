//! Seuils d'alerte sur les stats (ADR-0043, amendement) : à chaque minute reçue, les
//! `alert_minutes` dernières minutes du serveur sont relues ; si toutes dépassent le seuil de
//! CPU ou de mémoire des paramètres, l'alerte s'active et `server.cpu_high` ou
//! `server.memory_high` part, une seule fois. Elle se désactive à la première minute sous le
//! seuil bas (le seuil moins [`ALERT_RESET_GAP`] points), et peut alors repartir.

use serde_json::json;
use sqlx::PgPool;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use super::link::server_ref;
use crate::notifications::application as notifications;
use crate::notifications::domain::{Event, EventKind};
use crate::servers::infra::{AlertRepository, MetricPoint, MetricsRepository, ServerRepository};
use crate::settings::infra::SettingsRepository;

/// Points de pourcentage sous le seuil à partir desquels une alerte se réarme.
pub const ALERT_RESET_GAP: i32 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertKind {
    Cpu,
    Memory,
}

impl AlertKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Memory => "memory",
        }
    }

    fn event(self) -> EventKind {
        match self {
            Self::Cpu => EventKind::ServerCpuHigh,
            Self::Memory => EventKind::ServerMemoryHigh,
        }
    }
}

/// Ce que la dernière minute change à une alerte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Raise,
    Clear,
    Keep,
}

/// `ratios` : usage rapporté à l'allocation, minute par minute, la plus récente en dernier.
/// Alerte si les `minutes` dernières sont toutes au seuil ou au-dessus ; réarmement si la
/// dernière est sous le seuil bas ; sinon rien ne change.
pub fn decide(ratios: &[f64], threshold_percent: i32, minutes: usize) -> Decision {
    let Some(latest) = ratios.last() else {
        return Decision::Keep;
    };
    let high = f64::from(threshold_percent) / 100.0;
    let low = f64::from(threshold_percent - ALERT_RESET_GAP) / 100.0;
    let minutes = minutes.max(1);
    if ratios.len() >= minutes && ratios[ratios.len() - minutes..].iter().all(|r| *r >= high) {
        Decision::Raise
    } else if *latest < low {
        Decision::Clear
    } else {
        Decision::Keep
    }
}

fn ratio(value: i64, limit: f64) -> f64 {
    if limit > 0.0 {
        value as f64 / limit
    } else {
        0.0
    }
}

/// Évalue les deux seuils d'un serveur après la minute `at` (début de minute). Sans allocation
/// CPU ou mémoire connue, le seuil correspondant ne se déclenche jamais.
pub async fn evaluate(
    pool: &PgPool,
    server_id: Uuid,
    at: OffsetDateTime,
) -> Result<(), sqlx::Error> {
    let mut conn = pool.acquire().await?;
    let Some(server) = ServerRepository::get(&mut conn, server_id).await? else {
        return Ok(());
    };
    let settings = SettingsRepository::get(&mut conn).await?;
    let minutes = usize::try_from(settings.alert_minutes).unwrap_or(5).max(1);
    let since = at - Duration::minutes(i64::try_from(minutes).unwrap_or(5) - 1);
    let points: Vec<MetricPoint> = MetricsRepository::minutes(&mut conn, server_id, since)
        .await?
        .into_iter()
        .filter(|p| p.at <= at)
        .collect();
    if points.is_empty() {
        return Ok(());
    }
    let cpu_limit = f64::from(server.cpu_percent) * 10.0;
    let memory_default = f64::from(server.memory_mb) * 1024.0 * 1024.0;
    let cpu: Vec<f64> = points
        .iter()
        .map(|p| ratio(p.cpu_millis_avg, cpu_limit))
        .collect();
    let memory: Vec<f64> = points
        .iter()
        .map(|p| {
            let limit = if p.memory_limit_bytes > 0 {
                p.memory_limit_bytes as f64
            } else {
                memory_default
            };
            ratio(p.memory_bytes_avg, limit)
        })
        .collect();
    let checks = [
        (AlertKind::Cpu, settings.alert_cpu_percent, cpu),
        (AlertKind::Memory, settings.alert_memory_percent, memory),
    ];
    for (kind, threshold, ratios) in checks {
        match decide(&ratios, threshold, minutes) {
            Decision::Raise => {
                if AlertRepository::raise(&mut conn, server_id, kind.as_str()).await? {
                    let latest = ratios.last().copied().unwrap_or(0.0);
                    let event = Event::now(kind.event())
                        .server(server_ref(&server))
                        .details(json!({
                            "percent": (latest * 100.0).round() as u64,
                            "threshold_percent": threshold,
                            "minutes": minutes,
                        }));
                    notifications::emit_detached(pool, event);
                }
            }
            Decision::Clear => {
                AlertRepository::clear(&mut conn, server_id, kind.as_str()).await?;
            }
            Decision::Keep => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_alert_needs_every_minute_of_the_window_above_the_threshold() {
        assert_eq!(decide(&[0.95, 0.95], 90, 3), Decision::Keep);
        assert_eq!(decide(&[0.95, 0.85, 0.95], 90, 3), Decision::Keep);
        assert_eq!(decide(&[0.5, 0.9, 0.95, 0.91], 90, 3), Decision::Raise);
        assert_eq!(decide(&[], 90, 3), Decision::Keep);
    }

    #[test]
    fn an_alert_rearms_under_the_low_threshold_only() {
        // Entre les deux seuils : rien ne change.
        assert_eq!(decide(&[0.95, 0.75], 90, 1), Decision::Keep);
        assert_eq!(decide(&[0.95, 0.69], 90, 1), Decision::Clear);
        assert_eq!(decide(&[0.95], 90, 1), Decision::Raise);
    }
}
