//! `GET /api/daemon` : poignée de main WebSocket, réservée aux nœuds enrôlés.

use axum::Extension;
use axum::extract::State;
use axum::extract::ws::WebSocketUpgrade;
use axum::http::StatusCode;
use axum::response::Response;
use kuro_protocol::limits::MAX_MESSAGE_BYTES;
use tracing::Instrument;

use super::listener::ListenerState;
use super::{PeerCertificate, session};
use crate::http::error::{ErrorCode, error_response};
use crate::nodes::domain::NodeStatus;

pub(super) async fn daemon_socket(
    State(state): State<ListenerState>,
    Extension(peer): Extension<PeerCertificate>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let Some(fingerprint) = peer.0 else {
        return error_response(
            StatusCode::UNAUTHORIZED,
            ErrorCode::ClientCertificateRequired,
        );
    };
    let node = match state.link.presence.identify(&fingerprint).await {
        Ok(Some(node)) if node.status == NodeStatus::Revoked => {
            tracing::warn!(node = %node.name, "revoked node refused");
            return error_response(StatusCode::FORBIDDEN, ErrorCode::NodeRevoked);
        }
        Ok(Some(node)) => node,
        // Nœud supprimé (ADR-0046) : son daemon doit s'arrêter, pas réessayer sans fin.
        Ok(None) => match state.link.presence.certificate_revoked(&fingerprint).await {
            Ok(true) => {
                tracing::warn!(%fingerprint, "certificate of a deleted node refused");
                return error_response(StatusCode::FORBIDDEN, ErrorCode::NodeRevoked);
            }
            Ok(false) => {
                tracing::warn!(%fingerprint, "unknown node certificate");
                return error_response(StatusCode::FORBIDDEN, ErrorCode::UnknownNode);
            }
            Err(error) => {
                tracing::error!(%error, "failed to identify node");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::InternalError);
            }
        },
        Err(error) => {
            tracing::error!(%error, "failed to identify node");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::InternalError);
        }
    };

    // La session tourne plus tard dans sa propre tâche : on y propage le span de la connexion.
    let connection_span = tracing::Span::current();
    upgrade
        .max_message_size(MAX_MESSAGE_BYTES)
        .max_frame_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| {
            async move {
                session::run(socket, node, &state.link, &state.shutdown).await;
            }
            .instrument(connection_span)
        })
}
