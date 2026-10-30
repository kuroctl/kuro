use crate::http::extract::Json;
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    /// Toujours `ok` si le processus répond.
    #[schema(example = "ok")]
    pub status: &'static str,
}

/// Sonde de vivacité, sans authentification.
#[utoipa::path(
    get,
    path = "/api/health",
    tag = "system",
    responses((status = 200, description = "Instance vivante", body = HealthResponse))
)]
pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}
