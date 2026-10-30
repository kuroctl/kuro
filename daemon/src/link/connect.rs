//! Ouverture du lien : TCP → TLS 1.3 avec certificat client → WebSocket.

use std::time::Duration;

use kuro_protocol::limits::MAX_MESSAGE_BYTES;
use rustls::pki_types::ServerName;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use tokio_tungstenite::tungstenite::http::Uri;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::{WebSocketStream, tungstenite};

use super::{LinkError, tls};
use crate::config::Config;

pub type WsStream = WebSocketStream<TlsStream<TcpStream>>;

const STEP_TIMEOUT: Duration = Duration::from_secs(10);

/// Ouvre une connexion authentifiée, sans envoyer de message applicatif.
pub async fn connect(config: &Config) -> Result<WsStream, LinkError> {
    let invalid_url = || LinkError::InvalidUrl(config.api.url.clone());
    let uri: Uri = config.api.url.parse().map_err(|_| invalid_url())?;
    let host = uri.host().ok_or_else(invalid_url)?;
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned();
    let port = uri.port_u16().unwrap_or(443);
    let server_name = ServerName::try_from(host.clone()).map_err(|_| invalid_url())?;

    let connector = TlsConnector::from(tls::client_config(&config.tls)?);

    let tcp = timeout(STEP_TIMEOUT, TcpStream::connect((host.as_str(), port)))
        .await
        .map_err(|_| LinkError::Timeout("connecting"))??;
    let tls = timeout(STEP_TIMEOUT, connector.connect(server_name, tcp))
        .await
        .map_err(|_| LinkError::Timeout("negotiating TLS"))??;
    let handshake = tokio_tungstenite::client_async_with_config(uri, tls, Some(websocket_config()));
    let (ws, _response) = timeout(STEP_TIMEOUT, handshake)
        .await
        .map_err(|_| LinkError::Timeout("opening websocket"))?
        .map_err(|error| match &error {
            tungstenite::Error::Http(response) if is_revoked(response) => LinkError::Revoked,
            _ => Box::new(error).into(),
        })?;
    Ok(ws)
}

/// Refus de la poignée de main pour un nœud révoqué : `403`, problem+json `code = node_revoked`.
fn is_revoked(response: &tungstenite::http::Response<Option<Vec<u8>>>) -> bool {
    response.status() == tungstenite::http::StatusCode::FORBIDDEN
        && response
            .body()
            .as_deref()
            .and_then(|body| serde_json::from_slice::<serde_json::Value>(body).ok())
            .is_some_and(|body| body["code"] == "node_revoked")
}

fn websocket_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_MESSAGE_BYTES))
}
