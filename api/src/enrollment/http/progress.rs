use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use super::EnrollmentState;
use crate::http::error::{ErrorCode, Problem, error_response};
use crate::http::extract::Json;
use crate::machines::application::{self as machines, MachinesError};

/// Chemin de l'annonce d'avancement d'une machine louée (ADR-0065).
pub const PROGRESS_PATH: &str = "/api/enroll/progress";

#[derive(Deserialize, utoipa::ToSchema)]
pub struct ProgressRequest {
    /// Jeton d'enrôlement du nœud de la machine (encore valide, même déjà utilisé).
    pub token: String,
    /// `install_started` ou `installed`.
    pub step: String,
    /// Échec de l'étape : sortie courte (300 caractères gardés).
    pub error: Option<String>,
}

/// Annonce de l'installeur d'une machine louée : étape atteinte, ou son échec.
#[utoipa::path(
    post,
    path = "/api/enroll/progress",
    tag = "enrollment",
    request_body = ProgressRequest,
    responses(
        (status = 204, description = "Étape notée"),
        (status = 401, description = "Jeton inconnu ou expiré, ou nœud sans machine", body = Problem),
        (status = 422, description = "Étape inconnue", body = Problem),
    )
)]
pub async fn announce(
    State(state): State<EnrollmentState>,
    Json(request): Json<ProgressRequest>,
) -> Response {
    let result = machines::announce(
        &state.pool,
        &request.token,
        &request.step,
        request.error.as_deref(),
    )
    .await;
    match result {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => {
            tracing::warn!("machine progress rejected: invalid token");
            error_response(StatusCode::UNAUTHORIZED, ErrorCode::InvalidToken)
        }
        Err(MachinesError::Invalid(field)) => Problem::invalid(field).into_response(),
        Err(error) => {
            tracing::error!(%error, "machine progress failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::InternalError)
        }
    }
}
