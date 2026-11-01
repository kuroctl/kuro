//! Contexte d'audit d'une requête : adresse du pair, User-Agent, identifiant de requête.

use std::convert::Infallible;
use std::net::SocketAddr;

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::header::USER_AGENT;
use axum::http::request::Parts;

use super::policy::ClientIp;
use crate::audit::domain::{Actor, AuditContext};

/// En-tête posé par `SetRequestIdLayer` et renvoyé au client.
pub const REQUEST_ID_HEADER: &str = "x-request-id";
/// Longueur conservée des en-têtes libres (User-Agent), pour ne pas gonfler le journal.
const MAX_HEADER_CHARS: usize = 256;

/// Contexte anonyme de la requête ; l'acteur est précisé une fois authentifié.
#[derive(Debug, Clone)]
pub struct RequestContext(pub AuditContext);

impl<S: Send + Sync> FromRequestParts<S> for RequestContext {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(request_context(parts)))
    }
}

pub fn request_context(parts: &Parts) -> AuditContext {
    let header = |name: &str| {
        parts
            .headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.chars().take(MAX_HEADER_CHARS).collect::<String>())
    };
    // Adresse établie derrière les proxys de confiance (`http.trusted_proxies`, ADR-0008) ;
    // à défaut, le pair TCP.
    let ip = parts
        .extensions
        .get::<ClientIp>()
        .map(|ClientIp(ip)| *ip)
        .or_else(|| {
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|ConnectInfo(addr)| addr.ip())
        });
    AuditContext {
        actor: Actor::anonymous(),
        ip: ip.map(|ip| ip.to_string()),
        user_agent: header(USER_AGENT.as_str()),
        request_id: header(REQUEST_ID_HEADER),
        agent: None,
        owner: None,
    }
}
