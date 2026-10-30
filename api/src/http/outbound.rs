//! Client HTTPS sortant commun aux intégrations (sources de contenu, Cloudflare, métadonnées
//! OAuth) : rustls avec les autorités publiques, configuration TLS construite une fois, HTTP/1.1,
//! réponse bornée en taille et en durée. Les politiques propres à chaque usage (hôtes autorisés,
//! adresses publiques seulement, cache) restent chez l'appelant.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Bytes;
use hyper::{Request, StatusCode};
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, RootCertStore};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OutboundError {
    #[error("timed out")]
    Timeout,
    #[error("{0}")]
    Transport(String),
}

/// Où se connecter : un nom résolu par le système, ou une adresse déjà vérifiée (l'appelant
/// a contrôlé qu'elle est publique et ne la résout pas une seconde fois).
#[derive(Debug, Clone)]
pub enum Target {
    Host(String, u16),
    Address(SocketAddr),
}

#[derive(Debug, Clone)]
pub struct Https {
    config: Arc<ClientConfig>,
}

impl Https {
    /// Autorités publiques (`webpki-roots`), TLS 1.2 et 1.3, HTTP/1.1.
    pub fn new() -> Result<Self, rustls::Error> {
        let roots = RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        };
        let mut config = ClientConfig::builder_with_provider(kuro_runtime::tls::provider())
            .with_safe_default_protocol_versions()?
            .with_root_certificates(roots)
            .with_no_client_auth();
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok(Self {
            config: Arc::new(config),
        })
    }

    /// Envoie `request` à `server_name` (certificat attendu) joint par `target`, dans `timeout`
    /// au plus ; la réponse est lue jusqu'à `max_bytes`, au-delà c'est une erreur.
    pub async fn send(
        &self,
        server_name: &str,
        target: Target,
        request: Request<Full<Bytes>>,
        max_bytes: usize,
        timeout: Duration,
    ) -> Result<(StatusCode, Bytes), OutboundError> {
        tokio::time::timeout(
            timeout,
            self.exchange(server_name, target, request, max_bytes),
        )
        .await
        .map_err(|_| OutboundError::Timeout)?
    }

    async fn exchange(
        &self,
        server_name: &str,
        target: Target,
        request: Request<Full<Bytes>>,
        max_bytes: usize,
    ) -> Result<(StatusCode, Bytes), OutboundError> {
        let transport = |e: &dyn std::fmt::Display| OutboundError::Transport(e.to_string());
        let server_name =
            ServerName::try_from(server_name.to_owned()).map_err(|e| transport(&e))?;
        let tcp = match target {
            Target::Host(host, port) => TcpStream::connect((host.as_str(), port)).await,
            Target::Address(addr) => TcpStream::connect(addr).await,
        }
        .map_err(|e| transport(&e))?;
        let tls = TlsConnector::from(Arc::clone(&self.config))
            .connect(server_name, tcp)
            .await
            .map_err(|e| transport(&e))?;
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
            .await
            .map_err(|e| transport(&e))?;
        tokio::spawn(async move {
            if let Err(error) = connection.await {
                tracing::debug!(%error, "outbound https connection ended with an error");
            }
        });
        let response = sender
            .send_request(request)
            .await
            .map_err(|e| transport(&e))?;
        let status = response.status();
        let bytes = Limited::new(response.into_body(), max_bytes)
            .collect()
            .await
            .map_err(|e| transport(&e))?
            .to_bytes();
        Ok((status, bytes))
    }
}
