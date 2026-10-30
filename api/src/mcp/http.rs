//! `/mcp` : transport Streamable HTTP de rmcp, derrière l'authentification des agents.
//!
//! Sans jeton valide : 401 et `WWW-Authenticate` qui désigne les métadonnées de la ressource
//! (RFC 9728), point de départ du flux OAuth du client. Avec un jeton : l'agent et son contexte
//! d'audit sont posés dans les extensions de la requête, que les outils relisent.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{FromRef, Request};
use axum::http::StatusCode;
use axum::http::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use tower::ServiceExt;

use super::server::{Caller, KuroMcp, McpState};
use crate::audit::domain::AgentRef;
use crate::http::context::RequestContext;
use crate::http::error::internal_error;
use crate::oauth::application::{OAuthState, authenticate};
use crate::oauth::http::{Enabled, challenge};

/// Taille maximale d'une requête MCP (fichiers d'un brouillon compris).
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

/// Route `/mcp`, montée seulement si le serveur d'autorisation est configuré.
pub fn routes<S>(state: McpState, oauth: &OAuthState) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    Option<OAuthState>: FromRef<S>,
{
    // Sans session (protocole 2026-07-28, et en mode sans état pour les versions antérieures) :
    // chaque requête porte son jeton, rien n'est gardé en mémoire entre deux appels.
    // L'authentification par jeton rend la vérification de `Host` inutile ; une page web ne
    // peut appeler `/mcp` que depuis le panel.
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_max_request_body_bytes(MAX_BODY_BYTES)
        .disable_allowed_hosts()
        .with_allowed_origins([origin_of(&oauth.panel_url)])
        .with_sse_keep_alive(None);
    let service = StreamableHttpService::new(
        move || Ok(KuroMcp::new(state.clone())),
        Arc::new(NeverSessionManager::default()),
        config,
    );
    Router::new().route(
        "/mcp",
        any(
            move |enabled: Enabled, context: RequestContext, request: Request| {
                handle(service.clone(), enabled, context, request)
            },
        ),
    )
}

/// Origine RFC 6454 avec son port explicite (`https://panel.example.com:443`), comme l'attend
/// la vérification d'`Origin` de rmcp.
fn origin_of(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("https", url));
    let authority = rest.split('/').next().unwrap_or(rest);
    // `[::1]` se termine par `]` : seul un suffixe numérique après le dernier `:` est un port.
    let has_port = authority
        .rsplit_once(':')
        .is_some_and(|(_, port)| port.parse::<u16>().is_ok());
    if has_port {
        format!("{scheme}://{authority}")
    } else {
        let port = if scheme == "http" { 80 } else { 443 };
        format!("{scheme}://{authority}:{port}")
    }
}

async fn handle(
    service: StreamableHttpService<KuroMcp, NeverSessionManager>,
    Enabled(oauth): Enabled,
    RequestContext(context): RequestContext,
    mut request: Request,
) -> Response {
    let token = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::to_owned);
    let agent = match token {
        Some(token) => authenticate(&oauth.pool, &token).await,
        None => Ok(None),
    };
    let agent = match agent {
        Ok(Some(agent)) => agent,
        Ok(None) => {
            return (
                StatusCode::UNAUTHORIZED,
                [(WWW_AUTHENTICATE, challenge(&oauth, None))],
            )
                .into_response();
        }
        Err(error) => return internal_error(&error, "mcp authentication failed"),
    };
    let context = context.as_agent(
        agent.principal.user_id,
        AgentRef {
            client_id: agent.client_id.clone(),
            grant_id: agent.grant_id,
        },
    );
    crate::http::trace::record_actor(&context.actor);
    request.extensions_mut().insert(Caller {
        permissions: agent.principal.permissions,
        context,
    });
    match service.oneshot(request).await {
        Ok(response) => response.map(Body::new),
        Err(never) => match never {},
    }
}

#[cfg(test)]
mod tests {
    use super::origin_of;

    #[test]
    fn origins_carry_an_explicit_port() {
        assert_eq!(
            origin_of("https://panel.example.com"),
            "https://panel.example.com:443"
        );
        assert_eq!(origin_of("http://localhost:5173"), "http://localhost:5173");
        assert_eq!(
            origin_of("https://panel.example.com/sub"),
            "https://panel.example.com:443"
        );
    }
}
