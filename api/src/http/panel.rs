//! Panel compilé servi par l'API (ADR-0016) : même origine que l'API, donc cookie
//! `SameSite=Strict` et WebSocket de console sans configuration.
//!
//! Toute route inconnue hors `/api` renvoie `index.html` : le routage est fait par le navigateur.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::header::{
    CACHE_CONTROL, CONTENT_SECURITY_POLICY, REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderValue, Request};
use axum::response::{IntoResponse, Response};
use tower::ServiceExt;
use tower_http::services::{ServeDir, ServeFile};

use super::extract::not_found;

/// Aucun script en ligne ni d'ailleurs. Les styles en ligne restent permis : xterm.js et les
/// composants posent des styles calculés. La carte du monde (ADR-0021) charge son style, ses
/// tuiles, polices et icônes depuis OpenFreeMap ; son worker est servi par le panel (`'self'`).
const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
     img-src 'self' data: blob: https://tiles.openfreemap.org; font-src 'self' data:; \
     connect-src 'self' https://tiles.openfreemap.org; worker-src 'self'; object-src 'none'; \
     base-uri 'none'; form-action 'self'; frame-ancestors 'none'";

/// Fichiers à empreinte produits par Vite : leur contenu ne change jamais.
const ASSETS: &str = "/assets/";

#[derive(Debug, Clone)]
pub struct Panel {
    dir: PathBuf,
}

impl Panel {
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
        }
    }

    /// Fichier du panel, ou `index.html` ; une route `/api` inconnue reste une erreur JSON.
    pub async fn serve(&self, request: Request<Body>) -> Response {
        let path = request.uri().path().to_owned();
        if path == "/api" || path.starts_with("/api/") {
            return not_found().await;
        }
        let index = self.dir.join("index.html");
        let files = ServeDir::new(&self.dir).fallback(ServeFile::new(&index));
        let mut response = match files.oneshot(request).await {
            Ok(response) => response.into_response(),
            Err(infallible) => match infallible {},
        };
        let headers = response.headers_mut();
        let cache = if path.starts_with(ASSETS) {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };
        headers.insert(CACHE_CONTROL, HeaderValue::from_static(cache));
        headers.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP));
        headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
        headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
        response
    }
}
