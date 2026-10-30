//! Vérification de l'en-tête `Origin` : la page qui appelle est le panel (même hôte que l'API,
//! ou origine déclarée dans `http.allowed_origins`).
//!
//! Deux extracteurs, pour deux situations :
//! - [`CheckedOrigin`] : l'en-tête est exigé. Pour les requêtes qu'un navigateur envoie toujours
//!   avec lui (handshake WebSocket, formulaire soumis).
//! - [`BrowserOrigin`] : l'en-tête, s'il est présent, doit être autorisé ; absent, la requête
//!   vient d'un client hors navigateur (CLI, script) et passe. Un navigateur pose `Origin` sur
//!   tout `POST`, d'une autre origine comme de la même : une page tierce ne peut ni l'omettre ni
//!   le falsifier. Pour les routes d'authentification (ADR-0038) : un site sous le même domaine
//!   que le panel est « même site », `SameSite=Strict` ne le bloque pas.
//!
//! Un refus est journalisé (`access.denied`, raison `origin`) et répond `403`.

use axum::extract::{FromRef, FromRequestParts};
use axum::http::header::{HOST, ORIGIN};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde_json::json;
use sqlx::PgPool;

use super::context::request_context;
use super::error::{ErrorCode, error_response};
use super::policy::HttpPolicy;
use crate::audit::domain::{AuditAction, AuditEvent};

/// `Origin` présent et autorisé.
pub struct CheckedOrigin;

impl<S> FromRequestParts<S> for CheckedOrigin
where
    HttpPolicy: FromRef<S>,
    PgPool: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match check(parts, state, true).await {
            None => Ok(Self),
            Some(denied) => Err(denied),
        }
    }
}

/// `Origin` absent (client hors navigateur) ou autorisé.
pub struct BrowserOrigin;

impl<S> FromRequestParts<S> for BrowserOrigin
where
    HttpPolicy: FromRef<S>,
    PgPool: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match check(parts, state, false).await {
            None => Ok(Self),
            Some(denied) => Err(denied),
        }
    }
}

/// Réponse de refus, ou rien si l'origine passe.
async fn check<S>(parts: &Parts, state: &S, required: bool) -> Option<Response>
where
    HttpPolicy: FromRef<S>,
    PgPool: FromRef<S>,
{
    let origin = parts.headers.get(ORIGIN).and_then(|v| v.to_str().ok());
    let policy = HttpPolicy::from_ref(state);
    match origin {
        None if !required => return None,
        Some(origin) if origin_allowed(origin, &parts.headers, policy.allowed_origins()) => {
            return None;
        }
        _ => {}
    }
    let context = request_context(parts);
    let origin: Option<String> = origin.map(|o| o.chars().take(200).collect());
    let event = AuditEvent::denied(AuditAction::AccessDenied)
        .details(json!({ "reason": "origin", "origin": origin }));
    if let Err(error) =
        crate::audit::application::record(&PgPool::from_ref(state), &context, event).await
    {
        tracing::error!(%error, "failed to record an origin denial");
    }
    Some(error_response(StatusCode::FORBIDDEN, ErrorCode::Forbidden))
}

/// Origine du panel : une origine déclarée, ou l'hôte de l'API lui-même, en `https://` (en
/// `http://` pour une adresse locale seulement : développement). Le schéma compte, sinon une
/// page `http://` servie sur le même nom passerait.
pub fn origin_allowed(origin: &str, headers: &HeaderMap, allowed: &[String]) -> bool {
    if allowed.iter().any(|a| a.eq_ignore_ascii_case(origin)) {
        return true;
    }
    let Some(host) = headers.get(HOST).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let same_host = |scheme: &str| {
        origin
            .strip_prefix(scheme)
            .is_some_and(|o| o.eq_ignore_ascii_case(host))
    };
    same_host("https://") || (is_local_host(host) && same_host("http://"))
}

/// `localhost`, `127.0.0.1`, `[::1]`, avec ou sans port.
fn is_local_host(host: &str) -> bool {
    let name = host.strip_prefix('[').map_or_else(
        || host.rsplit_once(':').map_or(host, |(h, _)| h),
        |v6| v6.split(']').next().unwrap_or(v6),
    );
    matches!(name, "localhost" | "127.0.0.1" | "::1")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(host: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(HOST, host.parse().unwrap());
        headers
    }

    #[test]
    fn the_apis_own_origin_counts_with_its_scheme() {
        let allowed = vec!["https://panel.example.com".to_owned()];
        let api = headers("api.example.com");
        assert!(origin_allowed("https://panel.example.com", &api, &allowed));
        assert!(origin_allowed("https://api.example.com", &api, &allowed));
        // Le schéma est comparé tel quel (un navigateur l'écrit en minuscules), l'hôte sans casse.
        assert!(origin_allowed("https://API.EXAMPLE.COM", &api, &allowed));
        // Même hôte en clair : une page `http://` ne doit pas ouvrir la console.
        assert!(!origin_allowed("http://api.example.com", &api, &allowed));
        assert!(!origin_allowed("https://evil.example.com", &api, &allowed));
        assert!(!origin_allowed(
            "https://api.example.com.evil.net",
            &api,
            &allowed
        ));
        // Développement : adresse locale en `http://`, avec son port.
        assert!(origin_allowed(
            "http://localhost:5173",
            &headers("localhost:5173"),
            &[]
        ));
        assert!(origin_allowed(
            "http://127.0.0.1:8080",
            &headers("127.0.0.1:8080"),
            &[]
        ));
        assert!(origin_allowed(
            "http://[::1]:8080",
            &headers("[::1]:8080"),
            &[]
        ));
        assert!(!origin_allowed(
            "http://localhost:5173",
            &headers("localhost:5174"),
            &[]
        ));
    }
}
