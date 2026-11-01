//! Lien sortant vers l'API : `wss://` en mTLS, `hello` puis heartbeats, reconnexion automatique.

mod backoff;
mod connect;
mod host;
mod requests;
mod session;
mod tls;

use kuro_protocol::CodecError;
use kuro_protocol::api::ErrorCode;
use tokio_tungstenite::tungstenite;
use tokio_util::sync::CancellationToken;

pub use connect::{WsStream, connect};
pub use requests::{ApiRequestQueue, ApiRequests, REQUEST_TIMEOUT, RequestError, channel};

use crate::config::Config;
use crate::server::Servers;
use backoff::Backoff;

#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    #[error("invalid api url {0:?}")]
    InvalidUrl(String),
    #[error(transparent)]
    Tls(#[from] kuro_runtime::tls::TlsError),
    #[error("invalid TLS configuration: {0}")]
    Rustls(#[from] rustls::Error),
    #[error("connection failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("timed out while {0}")]
    Timeout(&'static str),
    #[error("websocket error: {0}")]
    WebSocket(#[from] Box<tungstenite::Error>),
    #[error(transparent)]
    Codec(#[from] CodecError),
    #[error("rejected by api ({code:?}): {message}")]
    Rejected { code: ErrorCode, message: String },
    #[error("unexpected message from api")]
    UnexpectedMessage,
    #[error("connection closed by api")]
    Closed,
    #[error("api did not answer within the heartbeat timeout")]
    Unresponsive,
    /// Définitif : le nœud doit être réenrôlé, se reconnecter ne servirait à rien.
    #[error("this node has been revoked or deleted: re-enroll it (`kuro-daemonctl enroll`)")]
    Revoked,
}

impl LinkError {
    pub(crate) fn from_api_error(error: kuro_protocol::api::ProtocolError) -> Self {
        if error.code == ErrorCode::NodeRevoked {
            Self::Revoked
        } else {
            Self::Rejected {
                code: error.code,
                message: error.message,
            }
        }
    }
}

/// Maintient le lien jusqu'à l'annulation de `shutdown`, en se reconnectant après chaque échec.
/// S'arrête définitivement (`Err(Revoked)`) si l'API signale que le nœud est révoqué.
///
/// Sans `servers`, le daemon refuse les commandes de serveurs (tests du lien sans Docker).
pub async fn run(
    config: &Config,
    servers: Option<&mut Servers>,
    shutdown: &CancellationToken,
) -> Result<(), LinkError> {
    run_with(config, servers, None, None, shutdown).await
}

/// Comme [`run`], avec les requêtes du daemon vers l'API (SFTP, ADR-0047) et le point d'accès
/// SFTP annoncé dans `hello`.
pub async fn run_with(
    config: &Config,
    mut servers: Option<&mut Servers>,
    mut requests: Option<&mut ApiRequestQueue>,
    sftp: Option<kuro_protocol::daemon::SftpEndpoint>,
    shutdown: &CancellationToken,
) -> Result<(), LinkError> {
    let mut backoff = Backoff::default();
    loop {
        let connected = tokio::select! {
            () = shutdown.cancelled() => return Ok(()),
            connected = connect(config) => connected,
        };
        let error = match connected {
            Ok(ws) => {
                let outcome = session::run(
                    ws,
                    shutdown,
                    &mut backoff,
                    servers.as_deref_mut(),
                    requests.as_deref_mut(),
                    sftp.clone(),
                )
                .await;
                if let Some(servers) = servers.as_deref_mut() {
                    servers.forget_watches();
                }
                match outcome {
                    Ok(()) => return Ok(()),
                    Err(error) => error,
                }
            }
            Err(error) => error,
        };
        if matches!(error, LinkError::Revoked) {
            tracing::error!(%error, "api link stopped");
            return Err(error);
        }

        let delay = backoff.next_delay();
        tracing::warn!(%error, retry_in_ms = delay.as_millis(), "api link lost");
        tokio::select! {
            () = shutdown.cancelled() => return Ok(()),
            () = tokio::time::sleep(delay) => {}
        }
    }
}
