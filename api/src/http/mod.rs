//! Couche HTTP transverse : composition des routeurs de domaine, OpenAPI, serveur.
//!
//! Chaque domaine expose ses propres routes et son propre fragment OpenAPI ;
//! ce module se contente de les assembler.

pub mod context;
pub mod error;
pub mod extract;
pub mod openapi;
pub mod origin;
pub mod outbound;
pub mod panel;
pub mod patch;
pub mod policy;
pub mod rate_limit;
pub mod server;
pub mod timeout;
pub mod trace;

use axum::Router;
use axum::extract::Request;
use axum::http::{HeaderValue, header};
use axum::middleware::{Next, from_fn, from_fn_with_state};
use axum::response::Response;
use axum::routing::get;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};

use crate::state::AppState;
use crate::{
    audit, content, dns, enrollment, iam, mcp, nodes, oauth, prefabs, servers, settings, studio,
    system,
};

/// Routeur complet de l'application :
/// - `/api/health`, `/api/openapi.json` : publics ;
/// - `/api/admin/v1/…` : espace admin (ADR-0007) ;
/// - `/api/client/v1/…` : espace client, à venir ;
/// - `/.well-known/oauth-…`, `/oauth/…` : serveur d'autorisation du MCP (ADR-0023).
///
/// Avec `http.panel_dir`, toute autre route sert le panel (ADR-0016).
pub fn router(state: AppState) -> Router {
    let panel = state.panel.clone();
    // `/mcp` seulement si le serveur d'autorisation est configuré (`mcp.public_url`).
    let agents = state.oauth.as_ref().map(|oauth| {
        mcp::http::routes(
            mcp::server::McpState {
                pool: state.pool.clone(),
                servers: state.servers.clone(),
                prefabs_dir: state.prefabs_dir.dir.clone(),
                stores: state.stores.clone(),
            },
            oauth,
        )
    });
    let admin = iam::http::routes()
        .merge(audit::http::routes())
        .merge(nodes::http::routes())
        .merge(enrollment::http::admin::routes())
        .merge(prefabs::http::routes())
        .merge(studio::http::routes())
        .merge(servers::http::routes())
        .merge(settings::http::routes())
        .merge(oauth::http::admin_routes())
        .merge(dns::http::routes())
        .merge(content::http::routes());
    let routes = Router::new()
        .nest("/api", system::http::routes())
        .nest("/api/admin/v1", admin)
        .route("/api/openapi.json", get(openapi::serve))
        .merge(oauth::http::public_routes())
        .merge(agents.unwrap_or_default())
        .method_not_allowed_fallback(extract::method_not_allowed);
    let routes = match panel {
        Some(panel) => routes.fallback(move |request| async move { panel.serve(request).await }),
        None => routes.fallback(extract::not_found),
    };
    // De l'extérieur vers l'intérieur : identifiant de requête, adresse cliente et politique,
    // journal de la requête, délai de réponse, puis les routes (dont les couches de limitation
    // de débit).
    let policy = state.http.clone();
    let routes = routes
        .with_state(state)
        .layer(from_fn(api_headers))
        .layer(from_fn_with_state(policy.clone(), timeout::request_timeout))
        .layer(from_fn(trace::log_request))
        .layer(from_fn_with_state(policy, policy::apply));
    with_request_id(routes)
}

/// En-têtes de sécurité des réponses de l'API (le panel pose les siens, dont la CSP) : pas de
/// détection de type, pas de `Referer` sortant, jamais dans un cadre.
async fn api_headers(request: Request, next: Next) -> Response {
    let api = is_api_path(request.uri().path());
    let mut response = next.run(request).await;
    if api {
        let headers = response.headers_mut();
        headers
            .entry(header::X_CONTENT_TYPE_OPTIONS)
            .or_insert(HeaderValue::from_static("nosniff"));
        headers
            .entry(header::REFERRER_POLICY)
            .or_insert(HeaderValue::from_static("no-referrer"));
        headers
            .entry(header::X_FRAME_OPTIONS)
            .or_insert(HeaderValue::from_static("DENY"));
    }
    response
}

/// Chemin de l'API (REST, OAuth, MCP), par opposition aux fichiers du panel.
pub(crate) fn is_api_path(path: &str) -> bool {
    ["/api/", "/oauth", "/.well-known", "/mcp"]
        .iter()
        .any(|prefix| path.starts_with(prefix))
}

/// Identifiant de requête (`x-request-id`) : repris s'il est fourni, sinon généré, renvoyé au
/// client et enregistré dans l'audit pour relier événements et logs techniques.
pub fn with_request_id(router: Router) -> Router {
    router
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
}
