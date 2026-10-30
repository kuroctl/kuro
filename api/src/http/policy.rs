//! Règles transverses du serveur HTTP : adresse réelle du client derrière les proxys de
//! confiance, limitation de débit des routes sans authentification, délai de requête.
//!
//! Posées une fois en tête du routeur ([`apply`]), elles sont lues par les couches de routes
//! ([`super::rate_limit`]) et par le contexte d'audit, via les extensions de la requête.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::HeaderMap;
use axum::middleware::Next;
use axum::response::Response;

use super::rate_limit::RateLimiter;
use crate::config::HttpConfig;

/// En-tête des proxys : adresses des clients successifs, la plus proche du serveur en dernier.
const FORWARDED_FOR: &str = "x-forwarded-for";

/// Adresse du client telle qu'établie par [`apply`] : le pair TCP, ou l'adresse transmise par
/// un proxy de confiance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientIp(pub IpAddr);

#[derive(Debug, Clone)]
pub struct HttpPolicy {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    trusted_proxies: Vec<IpAddr>,
    limiter: Option<RateLimiter>,
    request_timeout: Option<Duration>,
}

impl HttpPolicy {
    pub fn from_config(config: &HttpConfig) -> Self {
        Self::new(
            config.trusted_proxies.clone(),
            config.rate_limit_per_minute,
            config.request_timeout(),
        )
    }

    /// Pairs joints directement, sans proxy : seul le pair TCP compte (listener des daemons).
    pub fn direct(rate_limit_per_minute: u32) -> Self {
        Self::new(Vec::new(), rate_limit_per_minute, None)
    }

    fn new(
        trusted_proxies: Vec<IpAddr>,
        rate_limit_per_minute: u32,
        request_timeout: Option<Duration>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                trusted_proxies,
                limiter: (rate_limit_per_minute > 0)
                    .then(|| RateLimiter::new(rate_limit_per_minute, Duration::from_secs(60))),
                request_timeout,
            }),
        }
    }

    pub fn request_timeout(&self) -> Option<Duration> {
        self.inner.request_timeout
    }

    pub(super) fn limiter(&self) -> Option<&RateLimiter> {
        self.inner.limiter.as_ref()
    }

    /// Adresse du client : celle transmise par un proxy de confiance, sinon le pair lui-même.
    pub fn client_ip(&self, peer: IpAddr, headers: &HeaderMap) -> IpAddr {
        client_ip(peer, headers, &self.inner.trusted_proxies)
    }
}

/// Réglages par défaut de [`HttpConfig`] : proxys locaux de confiance, 30 requêtes par minute.
impl Default for HttpPolicy {
    fn default() -> Self {
        Self::from_config(&HttpConfig::default())
    }
}

/// Proxys de confiance par défaut : l'API n'écoute qu'en local, le proxy est sur la machine.
pub fn default_trusted_proxies() -> Vec<IpAddr> {
    vec![
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(Ipv6Addr::LOCALHOST),
    ]
}

/// Adresse du client d'après le pair et `X-Forwarded-For`. Un pair qui n'est pas un proxy de
/// confiance est le client, quoi qu'il annonce. Derrière un proxy de confiance, c'est la
/// dernière adresse de l'en-tête qui n'est pas elle-même un proxy de confiance (les précédentes
/// sont déclarées par le client, invérifiables). Sans en-tête exploitable : le pair.
pub fn client_ip(peer: IpAddr, headers: &HeaderMap, trusted: &[IpAddr]) -> IpAddr {
    if !trusted.contains(&peer) {
        return peer;
    }
    headers
        .get_all(FORWARDED_FOR)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .map(|hop| hop.parse::<IpAddr>().ok())
        .collect::<Vec<Option<IpAddr>>>()
        .into_iter()
        .rev()
        // Une adresse illisible dans la chaîne interrompt la remontée : on garde le pair.
        .take_while(Option::is_some)
        .flatten()
        .find(|hop| !trusted.contains(hop))
        .unwrap_or(peer)
}

/// Couche de tête : établit [`ClientIp`] et rend la politique disponible aux couches de routes.
pub async fn apply(State(policy): State<HttpPolicy>, mut request: Request, next: Next) -> Response {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| addr.ip());
    if let Some(peer) = peer {
        let ip = policy.client_ip(peer, request.headers());
        request.extensions_mut().insert(ClientIp(ip));
    }
    request.extensions_mut().insert(policy);
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(forwarded: &[&str]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for value in forwarded {
            headers.append(FORWARDED_FOR, value.parse().unwrap());
        }
        headers
    }

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn an_untrusted_peer_is_the_client_whatever_it_announces() {
        let trusted = default_trusted_proxies();
        assert_eq!(
            client_ip(ip("198.51.100.7"), &headers(&["203.0.113.9"]), &trusted),
            ip("198.51.100.7")
        );
    }

    #[test]
    fn behind_a_trusted_proxy_the_last_untrusted_hop_is_the_client() {
        let trusted = vec![ip("127.0.0.1"), ip("10.0.0.2")];
        // Client → proxy 10.0.0.2 → proxy local : les deux proxys s'effacent.
        assert_eq!(
            client_ip(
                ip("127.0.0.1"),
                &headers(&["203.0.113.9, 10.0.0.2"]),
                &trusted
            ),
            ip("203.0.113.9")
        );
        // Chaîne annoncée par le client : seule la dernière adresse non proxy compte.
        assert_eq!(
            client_ip(
                ip("127.0.0.1"),
                &headers(&["1.1.1.1", "203.0.113.9"]),
                &trusted
            ),
            ip("203.0.113.9")
        );
    }

    #[test]
    fn without_a_usable_header_the_peer_is_kept() {
        let trusted = default_trusted_proxies();
        assert_eq!(
            client_ip(ip("127.0.0.1"), &HeaderMap::new(), &trusted),
            ip("127.0.0.1")
        );
        assert_eq!(
            client_ip(ip("127.0.0.1"), &headers(&["unknown"]), &trusted),
            ip("127.0.0.1")
        );
        assert_eq!(
            client_ip(
                ip("127.0.0.1"),
                &headers(&["203.0.113.9, garbage"]),
                &trusted
            ),
            ip("127.0.0.1")
        );
        // Que des proxys : le pair.
        assert_eq!(
            client_ip(ip("::1"), &headers(&["127.0.0.1"]), &trusted),
            ip("::1")
        );
    }
}
