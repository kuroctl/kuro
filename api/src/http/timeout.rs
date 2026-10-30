//! Délai de réponse des requêtes : un traitement qui n'a pas rendu sa réponse à temps est
//! abandonné (`504 request_timeout`), pour qu'un nœud ou un service lent n'accumule pas des
//! requêtes sans fin. Les corps en flux (téléchargements) ne sont pas concernés : seule la
//! production de la réponse est chronométrée.
//!
//! Une route longue par nature (envoi de fichier, installation de modpack) s'en exempte avec
//! [`exempt`] : le drapeau est levé en cours de traitement, la couche de tête le constate à
//! l'échéance et laisse finir.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use super::error::{ErrorCode, Problem};
use super::policy::HttpPolicy;

/// Drapeau d'exemption de la requête en cours.
#[derive(Debug, Clone, Default)]
struct Exempt(Arc<AtomicBool>);

/// Couche de tête : borne le traitement à `http.request_timeout_secs`.
pub async fn request_timeout(
    State(policy): State<HttpPolicy>,
    mut request: Request,
    next: Next,
) -> Response {
    let Some(timeout) = policy.request_timeout() else {
        return next.run(request).await;
    };
    let exempt = Exempt::default();
    request.extensions_mut().insert(exempt.clone());
    let run = next.run(request);
    tokio::pin!(run);
    tokio::select! {
        response = &mut run => response,
        () = tokio::time::sleep(timeout) => {
            if exempt.0.load(Ordering::Acquire) {
                run.await
            } else {
                tracing::warn!(timeout_ms = timeout.as_millis(), "request timed out");
                Problem::new(StatusCode::GATEWAY_TIMEOUT, ErrorCode::RequestTimeout).into_response()
            }
        }
    }
}

/// Couche de route : la requête n'est pas soumise au délai global.
pub async fn exempt(request: Request, next: Next) -> Response {
    if let Some(exempt) = request.extensions().get::<Exempt>() {
        exempt.0.store(true, Ordering::Release);
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::Router;
    use axum::body::Body;
    use axum::http::Request;
    use axum::routing::get;
    use tower::ServiceExt;

    use super::*;
    use crate::config::HttpConfig;

    /// `/slow` et `/long` mettent 5 s ; seule `/long` est exemptée.
    fn app(timeout_secs: u64) -> Router {
        let policy = HttpPolicy::from_config(&HttpConfig {
            request_timeout_secs: timeout_secs,
            ..HttpConfig::default()
        });
        let late = || async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            "late"
        };
        Router::new()
            .route("/fast", get(|| async { "ok" }))
            .route("/slow", get(late))
            .route("/long", get(late).layer(axum::middleware::from_fn(exempt)))
            .layer(axum::middleware::from_fn_with_state(
                policy,
                request_timeout,
            ))
    }

    async fn status(app: &Router, path: &str) -> StatusCode {
        app.clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status()
    }

    #[tokio::test(start_paused = true)]
    async fn a_slow_handler_is_cut_unless_its_route_is_exempt() {
        let app = app(1);
        assert_eq!(status(&app, "/fast").await, StatusCode::OK);
        assert_eq!(status(&app, "/slow").await, StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(status(&app, "/long").await, StatusCode::OK);
    }

    #[tokio::test(start_paused = true)]
    async fn without_a_timeout_everything_finishes() {
        let app = app(0);
        assert_eq!(status(&app, "/slow").await, StatusCode::OK);
    }
}
