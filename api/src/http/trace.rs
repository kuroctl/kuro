//! Journal technique des requêtes : un span par requête (identifiant, méthode, chemin, adresse
//! cliente, acteur une fois authentifié) et une ligne à la réponse (statut, durée).
//!
//! Le chemin seul est journalisé, jamais la chaîne de requête ni les en-têtes : ni jeton ni donnée
//! personnelle superflue (ADR-0008). L'identifiant de requête est celui renvoyé au client et
//! inscrit dans le journal d'audit : il relie une ligne de log à ses événements métier. Les sondes
//! et les fichiers du panel passent en `debug` ; une erreur serveur passe en `error`.

use std::time::Instant;

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use tracing::{Instrument, field};

use super::context::REQUEST_ID_HEADER;
use super::is_api_path;
use super::policy::ClientIp;
use crate::audit::domain::Actor;

/// Couche à poser sous l'identifiant de requête et la politique d'adresse (`policy::apply`),
/// au-dessus de tout le reste.
pub async fn log_request(request: Request, next: Next) -> Response {
    let started = Instant::now();
    let path = request.uri().path().to_owned();
    let request_id = request
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let ip = request
        .extensions()
        .get::<ClientIp>()
        .map(|ClientIp(ip)| ip.to_string());
    let span = tracing::info_span!(
        "request",
        method = %request.method(),
        path = %path,
        request_id = %request_id,
        ip = ip.as_deref().unwrap_or_default(),
        actor = field::Empty,
    );
    let response = next.run(request).instrument(span.clone()).await;
    let status = response.status().as_u16();
    let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    match verbosity(&path, status) {
        Verbosity::Error => tracing::error!(parent: &span, status, latency_ms, "request failed"),
        Verbosity::Info => tracing::info!(parent: &span, status, latency_ms, "request"),
        Verbosity::Debug => tracing::debug!(parent: &span, status, latency_ms, "request"),
    }
    response
}

/// Acteur identifié par un extracteur : inscrit dans le span de la requête en cours, pour relier
/// les logs à l'audit sans attendre un événement métier.
pub fn record_actor(actor: &Actor) {
    let label = match actor.id {
        Some(id) => format!("{}:{id}", actor.kind.as_str()),
        None => actor.kind.as_str().to_owned(),
    };
    tracing::Span::current().record("actor", field::display(&label));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verbosity {
    Error,
    Info,
    Debug,
}

/// Niveau de la ligne : les sondes restent en `debug` (`/api/ready` journalise lui-même une
/// indisponibilité), une erreur serveur passe en `error`, une requête d'API en `info`, les
/// fichiers du panel en `debug`.
fn verbosity(path: &str, status: u16) -> Verbosity {
    if matches!(path, "/api/health" | "/api/ready") {
        return Verbosity::Debug;
    }
    if status >= 500 {
        return Verbosity::Error;
    }
    if is_api_path(path) {
        Verbosity::Info
    } else {
        Verbosity::Debug
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_requests_are_logged_and_probes_and_panel_files_are_not() {
        assert_eq!(verbosity("/api/admin/v1/servers", 200), Verbosity::Info);
        assert_eq!(verbosity("/api/admin/v1/servers", 404), Verbosity::Info);
        assert_eq!(verbosity("/oauth/token", 400), Verbosity::Info);
        assert_eq!(verbosity("/api/health", 200), Verbosity::Debug);
        assert_eq!(verbosity("/api/ready", 503), Verbosity::Debug);
        assert_eq!(verbosity("/assets/index.js", 200), Verbosity::Debug);
    }

    #[test]
    fn a_server_error_is_logged_as_an_error_except_on_a_probe() {
        assert_eq!(verbosity("/api/admin/v1/servers", 500), Verbosity::Error);
        assert_eq!(verbosity("/assets/index.js", 502), Verbosity::Error);
        assert_eq!(verbosity("/api/ready", 503), Verbosity::Debug);
    }
}
