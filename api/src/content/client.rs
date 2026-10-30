//! Client des providers de contenu : HTTPS commun ([`crate::http::outbound`]), réponses
//! bornées, cache de cinq minutes des réponses JSON. Partagé par Modrinth, CurseForge et Steam.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use http_body_util::Full;
use hyper::body::Bytes;
use hyper::header::{ACCEPT, CONTENT_TYPE, HOST, HeaderName, USER_AGENT};
use hyper::{Method, Request, StatusCode, Uri};
use serde_json::Value;

use super::provider::ProviderError;
use crate::http::outbound::{Https, OutboundError, Target};
use crate::validation::is_public;

const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(15);
const CACHE_TTL: Duration = Duration::from_secs(300);
const CACHE_ENTRIES: usize = 512;

/// En-tête ajouté à chaque requête (clé d'API) : jamais journalisé ni mis dans la clé du cache.
pub type Header = (HeaderName, String);

#[derive(Debug, Clone)]
pub struct JsonClient {
    /// Nom de la source, pour les messages d'erreur.
    name: &'static str,
    base: Uri,
    https: Https,
    cache: Arc<Mutex<HashMap<String, (Instant, Value)>>>,
}

impl JsonClient {
    pub fn new(name: &'static str, base_url: &str) -> Result<Self, ProviderError> {
        let base: Uri = base_url
            .trim_end_matches('/')
            .parse()
            .map_err(|_| ProviderError::Unavailable(format!("invalid {name} url")))?;
        if base.scheme_str() != Some("https") {
            return Err(ProviderError::Unavailable(format!(
                "{name} url must be https"
            )));
        }
        Ok(Self {
            name,
            base,
            https: Https::new().map_err(|e| ProviderError::Unavailable(e.to_string()))?,
            cache: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Requête vers l'API, mise en cache (lectures et recherches idempotentes). Un statut 401 ou
    /// 403 donne `Unauthorized`, 404 `NotFound`.
    pub async fn call(
        &self,
        method: Method,
        path_and_query: &str,
        body: Option<Value>,
        header: Option<&Header>,
    ) -> Result<Value, ProviderError> {
        let key = format!(
            "{method} {path_and_query} {}",
            body.as_ref().map(Value::to_string).unwrap_or_default()
        );
        if let Some((at, value)) = self.cache().get(&key)
            && at.elapsed() < CACHE_TTL
        {
            return Ok(value.clone());
        }
        let value = self.send(method, path_and_query, body, header).await?;
        let mut cache = self.cache();
        if cache.len() >= CACHE_ENTRIES {
            cache.retain(|_, (at, _)| at.elapsed() < CACHE_TTL);
            if cache.len() >= CACHE_ENTRIES {
                cache.clear();
            }
        }
        cache.insert(key, (Instant::now(), value.clone()));
        Ok(value)
    }

    /// Oublie les réponses gardées (clé d'API changée).
    pub fn clear_cache(&self) {
        self.cache().clear();
    }

    fn cache(&self) -> std::sync::MutexGuard<'_, HashMap<String, (Instant, Value)>> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }

    async fn send(
        &self,
        method: Method,
        path_and_query: &str,
        body: Option<Value>,
        header: Option<&Header>,
    ) -> Result<Value, ProviderError> {
        let name = self.name;
        let base_path = self.base.path().trim_end_matches('/');
        let uri: Uri = format!(
            "{}://{}{base_path}{path_and_query}",
            self.base.scheme_str().unwrap_or("https"),
            self.base.authority().map_or("", |a| a.as_str())
        )
        .parse()
        .map_err(|_| ProviderError::Unavailable(format!("invalid {name} url")))?;
        let (status, bytes) = self
            .request(method, &uri, body, header, MAX_RESPONSE_BYTES)
            .await?;
        match status {
            StatusCode::OK => serde_json::from_slice(&bytes)
                .map_err(|e| ProviderError::Unavailable(e.to_string())),
            StatusCode::NOT_FOUND => Err(ProviderError::NotFound),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(ProviderError::Unauthorized),
            other => Err(ProviderError::Unavailable(format!(
                "{name} answered {other}"
            ))),
        }
    }

    /// Requête HTTPS vers `uri`, réponse de `max_bytes` au plus.
    pub async fn request(
        &self,
        method: Method,
        uri: &Uri,
        body: Option<Value>,
        header: Option<&Header>,
        max_bytes: usize,
    ) -> Result<(StatusCode, Bytes), ProviderError> {
        let name = self.name;
        let unavailable = |e: &dyn std::fmt::Display| ProviderError::Unavailable(e.to_string());
        if uri.scheme_str() != Some("https") {
            return Err(unavailable(&"https only"));
        }
        let host = uri
            .host()
            .ok_or_else(|| unavailable(&"no host"))?
            .to_owned();
        let port = uri.port_u16().unwrap_or(443);
        // Les listes d'hôtes des sources protègent déjà ; une résolution vers une adresse privée
        // (DNS détourné) est tout de même refusée, et la connexion va à l'adresse vérifiée.
        let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host.as_str(), port))
            .await
            .map_err(|e| unavailable(&e))?
            .collect();
        let Some(addr) = addrs.first().copied() else {
            return Err(unavailable(&"no address"));
        };
        if !addrs.iter().all(|a| is_public(a.ip())) {
            return Err(ProviderError::Unavailable(format!(
                "{name} resolves to a non-public address"
            )));
        }

        let payload = body.map(|b| b.to_string()).unwrap_or_default();
        let mut request = Request::builder()
            .method(method)
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
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "application/json");
        if let Some((name, value)) = header {
            request = request.header(name, value);
        }
        let request = request
            .body(Full::new(Bytes::from(payload)))
            .map_err(|e| unavailable(&e))?;
        self.https
            .send(&host, Target::Address(addr), request, max_bytes, TIMEOUT)
            .await
            .map_err(|error| match error {
                OutboundError::Timeout => ProviderError::Unavailable(format!("{name} timed out")),
                OutboundError::Transport(message) => ProviderError::Unavailable(message),
            })
    }
}

/// Encode une valeur de paramètre de requête.
pub fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(byte));
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}
