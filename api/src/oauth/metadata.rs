//! Lecture du document de métadonnées d'un client, à l'URL qui lui sert de `client_id`.
//!
//! L'URL vient d'un inconnu : la lecture ne vise que des adresses publiques (vérifiées après
//! résolution, la connexion se fait à l'adresse vérifiée), ne suit aucune redirection, et reste
//! bornée en taille et en durée.

use std::time::Duration;

use futures_util::future::BoxFuture;
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::header::{ACCEPT, HOST, USER_AGENT};
use hyper::{Request, StatusCode, Uri};

use super::domain::{MAX_METADATA_BYTES, is_public, metadata_url_allowed};
use crate::http::outbound::{Https, OutboundError, Target};

const FETCH_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("client metadata url refused: {0}")]
    Refused(&'static str),
    #[error("client metadata unavailable: {0}")]
    Unavailable(String),
}

impl From<OutboundError> for FetchError {
    fn from(error: OutboundError) -> Self {
        Self::Unavailable(error.to_string())
    }
}

/// Source des documents de métadonnées (simulée dans les tests).
pub trait MetadataFetcher: Send + Sync + std::fmt::Debug {
    fn fetch(&self, url: &str) -> BoxFuture<'_, Result<Vec<u8>, FetchError>>;
}

/// Lecture HTTPS réelle, racines de confiance publiques.
#[derive(Debug, Clone)]
pub struct HttpsFetcher {
    https: Https,
}

impl HttpsFetcher {
    pub fn new() -> Result<Self, rustls::Error> {
        Ok(Self {
            https: Https::new()?,
        })
    }
}

impl MetadataFetcher for HttpsFetcher {
    fn fetch(&self, url: &str) -> BoxFuture<'_, Result<Vec<u8>, FetchError>> {
        let url = url.to_owned();
        Box::pin(async move { self.get(&url).await })
    }
}

impl HttpsFetcher {
    async fn get(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        let unavailable = |e: &dyn std::fmt::Display| FetchError::Unavailable(e.to_string());
        if !metadata_url_allowed(url) {
            return Err(FetchError::Refused("https url with a path required"));
        }
        let uri: Uri = url.parse().map_err(|e| unavailable(&e))?;
        let host = uri
            .host()
            .ok_or(FetchError::Refused("no host"))?
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_owned();
        let port = uri.port_u16().unwrap_or(443);

        // Toutes les adresses du nom doivent être publiques : on se connecte à l'une d'elles,
        // jamais à un nom résolu une seconde fois (rebinding DNS).
        let addrs: Vec<_> = tokio::net::lookup_host((host.as_str(), port))
            .await
            .map_err(|e| unavailable(&e))?
            .collect();
        let Some(addr) = addrs.first().copied() else {
            return Err(unavailable(&"no address"));
        };
        if !addrs.iter().all(|a| is_public(a.ip())) {
            return Err(FetchError::Refused("host resolves to a non-public address"));
        }

        let request = Request::get(uri.path_and_query().map_or("/", |p| p.as_str()))
            .header(HOST, uri.authority().map_or(host.as_str(), |a| a.as_str()))
            .header(
                USER_AGENT,
                concat!(
                    "kuro-api/",
                    env!("CARGO_PKG_VERSION"),
                    " (oauth client metadata)"
                ),
            )
            .header(ACCEPT, "application/json")
            .body(Full::new(Bytes::new()))
            .map_err(|e| unavailable(&e))?;
        let (status, bytes) = self
            .https
            .send(
                &host,
                Target::Address(addr),
                request,
                MAX_METADATA_BYTES,
                FETCH_TIMEOUT,
            )
            .await?;
        if status != StatusCode::OK {
            return Err(FetchError::Unavailable(format!("answered {status}")));
        }
        Ok(bytes.to_vec())
    }
}
