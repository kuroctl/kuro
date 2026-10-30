//! Routes HTTP du domaine `enrollment` :
//! - `routes` : `POST /api/enroll`, servi par le listener daemon (port 8443) ;
//! - `admin` : création des nœuds et de leurs tokens, dans l'espace admin.

pub mod admin;
mod enroll;

use std::sync::Arc;

use axum::Router;
use axum::extract::{DefaultBodyLimit, FromRef};
use axum::routing::post;
use kuro_protocol::enroll::ENROLL_PATH;
use sqlx::PgPool;
use utoipa::OpenApi;

use crate::pki::domain::CertificateAuthority;

/// Une CSR PEM tient en quelques kilo-octets.
const MAX_BODY_BYTES: usize = 64 * 1024;

/// Dépendances des handlers d'enrôlement.
#[derive(Debug, Clone)]
pub struct EnrollmentState {
    pub pool: PgPool,
    pub ca: Arc<CertificateAuthority>,
    pub certificate_validity: time::Duration,
}

pub use admin::EnrollmentSettings;

#[derive(OpenApi)]
#[openapi(
    paths(enroll::enroll, admin::create, admin::reissue),
    tags((name = "enrollment", description = "Enrôlement des nœuds, sur le listener daemon (port 8443, TLS de la CA privée)"))
)]
pub struct EnrollmentApi;

pub fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    EnrollmentState: FromRef<S>,
{
    Router::new()
        .route(ENROLL_PATH, post(enroll::enroll))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        // Sans authentification (le token est dans le corps) : débit limité par adresse.
        .route_layer(axum::middleware::from_fn(
            crate::http::rate_limit::limit_unauthenticated,
        ))
}
