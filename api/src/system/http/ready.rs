//! Sonde de disponibilité : base joignable et listener daemon en marche.

use axum::extract::State;
use axum::http::StatusCode;
use serde::Serialize;
use sqlx::PgPool;
use utoipa::ToSchema;

use crate::http::extract::Json;
use crate::system::Readiness;

#[derive(Debug, Serialize, ToSchema)]
pub struct ReadyResponse {
    /// `ready` quand toutes les vérifications passent, `not_ready` sinon (statut 503).
    #[schema(example = "ready")]
    pub status: &'static str,
    /// Base de données joignable : `ok` ou `unavailable`.
    #[schema(example = "ok")]
    pub database: &'static str,
    /// Listener des daemons en marche : `ok` ou `down`.
    #[schema(example = "ok")]
    pub daemon_listener: &'static str,
}

/// Sonde de disponibilité, sans authentification : 200 si l'instance peut servir, 503 sinon.
/// À brancher sur le répartiteur de charge ; `/api/health` dit seulement que le processus répond.
#[utoipa::path(
    get,
    path = "/api/ready",
    tag = "system",
    responses(
        (status = 200, description = "Instance prête", body = ReadyResponse),
        (status = 503, description = "Instance indisponible", body = ReadyResponse)
    )
)]
pub async fn ready(
    State(pool): State<PgPool>,
    State(readiness): State<Readiness>,
) -> (StatusCode, Json<ReadyResponse>) {
    let report = readiness.check(&pool).await;
    let status = if report.ready() {
        StatusCode::OK
    } else {
        tracing::warn!(
            database = report.database,
            daemon_listener = report.daemon_listener,
            "instance not ready"
        );
        StatusCode::SERVICE_UNAVAILABLE
    };
    let response = ReadyResponse {
        status: if report.ready() { "ready" } else { "not_ready" },
        database: if report.database { "ok" } else { "unavailable" },
        daemon_listener: if report.daemon_listener { "ok" } else { "down" },
    };
    (status, Json(response))
}
