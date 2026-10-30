//! `GET /servers/{id}/metrics` : historique des stats (ADR-0043).

use axum::extract::State;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::error_code::{ErrorCode, FieldError};
use crate::http::error::{ApiError, Problem};
use crate::http::extract::{Json, Path, Query};
use crate::iam::application::IamState;
use crate::iam::domain::Permission;
use crate::iam::http::AdminSession;
use crate::servers::application::ServersState;
use crate::servers::application::metrics::{self, Range};

#[derive(Debug, Deserialize, IntoParams)]
pub struct MetricsQuery {
    /// `24h` (à la minute, par défaut), `7d` ou `90d` (à l'heure).
    #[serde(default)]
    pub range: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MetricPointDto {
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub at: OffsetDateTime,
    /// Mesures agrégées dans ce point.
    pub samples: i32,
    /// 1000 = un cœur plein.
    pub cpu_millis_avg: i64,
    pub cpu_millis_max: i64,
    pub memory_bytes_avg: i64,
    pub memory_bytes_max: i64,
    pub memory_limit_bytes: i64,
    /// Octets de la période, pas des cumuls.
    pub network_rx_bytes: i64,
    pub network_tx_bytes: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MetricsDto {
    pub range: &'static str,
    /// Pas entre deux points, en secondes (60 ou 3600).
    pub step_secs: u32,
    pub points: Vec<MetricPointDto>,
}

/// Historique CPU, mémoire et réseau d'un serveur, envoyé par son nœud une fois par minute
/// (ADR-0043) : 24 h à la minute, 7 j ou 90 j à l'heure. Vide tant que le nœud n'a rien envoyé
/// (daemon antérieur). Permission : `servers.read`.
#[utoipa::path(get, path = "/api/admin/v1/servers/{id}/metrics", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), MetricsQuery),
    responses((status = 200, body = MetricsDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 422, description = "Plage inconnue", body = Problem)))]
pub async fn history(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Query(query): Query<MetricsQuery>,
) -> Result<Json<MetricsDto>, ApiError> {
    session.require(&iam, Permission::ServersRead).await?;
    let range = match query.range.as_deref() {
        None => Range::Day,
        Some(text) => Range::parse(text).ok_or_else(|| {
            ApiError::from(Problem::invalid(FieldError::new(
                "range",
                ErrorCode::InvalidMetricsRange,
            )))
        })?,
    };
    let points = metrics::history(&state, id, range).await?;
    Ok(Json(MetricsDto {
        range: range.as_str(),
        step_secs: range.step_secs(),
        points: points
            .into_iter()
            .map(|p| MetricPointDto {
                at: p.at,
                samples: p.samples,
                cpu_millis_avg: p.cpu_millis_avg,
                cpu_millis_max: p.cpu_millis_max,
                memory_bytes_avg: p.memory_bytes_avg,
                memory_bytes_max: p.memory_bytes_max,
                memory_limit_bytes: p.memory_limit_bytes,
                network_rx_bytes: p.network_rx_bytes,
                network_tx_bytes: p.network_tx_bytes,
            })
            .collect(),
    }))
}
