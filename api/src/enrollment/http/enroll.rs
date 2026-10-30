use crate::http::extract::Json;
use axum::Extension;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use kuro_protocol::enroll::{EnrollRequest, EnrollResponse};

use super::EnrollmentState;
use crate::enrollment::application::{self, EnrollError};
use crate::http::context::RequestContext;
use crate::http::error::{ErrorCode, Problem, error_response};
use crate::ws::PeerCertificate;

/// Échange un token d'enrôlement et une CSR contre le certificat client du nœud.
///
/// Réservé aux hôtes pas encore enrôlés : une requête présentant un certificat client est refusée.
#[utoipa::path(
    post,
    path = "/api/enroll",
    tag = "enrollment",
    request_body = EnrollRequest,
    responses(
        (status = 200, description = "Nœud enrôlé", body = EnrollResponse),
        (status = 400, description = "CSR invalide (le token n'est pas consommé)", body = Problem),
        (status = 401, description = "Token inconnu, expiré ou déjà utilisé", body = Problem),
        (status = 403, description = "Certificat client présenté", body = Problem),
    )
)]
pub async fn enroll(
    State(state): State<EnrollmentState>,
    Extension(peer): Extension<PeerCertificate>,
    RequestContext(context): RequestContext,
    Json(request): Json<EnrollRequest>,
) -> Response {
    if peer.0.is_some() {
        return error_response(StatusCode::FORBIDDEN, ErrorCode::AlreadyEnrolled);
    }

    let result = application::enroll(
        &state.pool,
        &context,
        &state.ca,
        &request.token,
        &request.csr_pem,
        state.certificate_validity,
    )
    .await;

    match result {
        Ok(enrollment) => Json(EnrollResponse {
            node_id: enrollment.node.id.as_uuid(),
            node_name: enrollment.node.name,
            cert_pem: enrollment.cert_pem,
            ca_pem: enrollment.ca_pem,
        })
        .into_response(),
        Err(EnrollError::InvalidToken) => {
            tracing::warn!("enrollment rejected: invalid token");
            error_response(StatusCode::UNAUTHORIZED, ErrorCode::InvalidToken)
        }
        Err(EnrollError::InvalidCsr(error)) => {
            tracing::warn!(%error, "enrollment rejected: invalid csr");
            error_response(StatusCode::BAD_REQUEST, ErrorCode::InvalidCsr)
        }
        Err(error) => {
            tracing::error!(%error, "enrollment failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::InternalError)
        }
    }
}
