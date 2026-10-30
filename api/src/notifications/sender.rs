//! Sortie HTTPS des notifications : `POST` d'un corps JSON vers l'URL d'un canal. Derrière un
//! trait pour être simulée dans les tests.

use std::time::Duration;

use futures_util::future::BoxFuture;
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::header::{CONTENT_TYPE, HOST, USER_AGENT};
use hyper::{Method, Request, Uri};

use crate::http::outbound::{Https, Target};
use crate::validation::is_public;

const TIMEOUT: Duration = Duration::from_secs(10);
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

/// Requête prête à partir : URL, corps JSON et en-têtes propres au canal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    pub url: String,
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
}

/// Envoi d'une notification ; `Ok` porte le statut HTTP (2xx), `Err` la cause (code stable ou
/// message court, journalisé sur l'envoi).
pub trait WebhookSender: Send + Sync + std::fmt::Debug {
    fn send(&self, outgoing: Outgoing, allow_private: bool) -> BoxFuture<'_, Result<u16, String>>;
}

/// Client HTTPS réel (autorités publiques, HTTP/1.1). Les adresses privées sont refusées sauf
/// réglage explicite : un webhook ne doit pas servir à joindre le réseau interne de l'API.
#[derive(Debug, Clone)]
pub struct HttpSender {
    https: Https,
}

impl HttpSender {
    pub fn new() -> Result<Self, rustls::Error> {
        Ok(Self {
            https: Https::new()?,
        })
    }
}

impl WebhookSender for HttpSender {
    fn send(&self, outgoing: Outgoing, allow_private: bool) -> BoxFuture<'_, Result<u16, String>> {
        Box::pin(async move {
            let uri: Uri = outgoing.url.parse().map_err(|_| "invalid_url".to_owned())?;
            if uri.scheme_str() != Some("https") {
                return Err("https_only".to_owned());
            }
            let host = uri
                .host()
                .ok_or_else(|| "invalid_url".to_owned())?
                .to_owned();
            let port = uri.port_u16().unwrap_or(443);
            let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host.as_str(), port))
                .await
                .map_err(|e| format!("dns: {e}"))?
                .collect();
            let Some(addr) = addrs.first().copied() else {
                return Err("no_address".to_owned());
            };
            if !allow_private && !addrs.iter().all(|a| is_public(a.ip())) {
                return Err("private_target".to_owned());
            }
            let mut request = Request::builder()
                .method(Method::POST)
                .uri(uri.path_and_query().map_or("/", |p| p.as_str()))
                .header(HOST, uri.authority().map_or(host.as_str(), |a| a.as_str()))
                .header(
                    USER_AGENT,
                    concat!(
                        "kuro-api/",
                        env!("CARGO_PKG_VERSION"),
                        " (github.com/kuroctl/kuro)"
                    ),
                )
                .header(CONTENT_TYPE, "application/json");
            for (name, value) in &outgoing.headers {
                request = request.header(name.as_str(), value.as_str());
            }
            let request = request
                .body(Full::new(Bytes::from(outgoing.body)))
                .map_err(|e| e.to_string())?;
            let (status, _) = self
                .https
                .send(
                    &host,
                    Target::Address(addr),
                    request,
                    MAX_RESPONSE_BYTES,
                    TIMEOUT,
                )
                .await
                .map_err(|e| e.to_string())?;
            if status.is_success() {
                Ok(status.as_u16())
            } else {
                Err(format!("status_{}", status.as_u16()))
            }
        })
    }
}
