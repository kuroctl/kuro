//! Routes HTTP du journal d'audit, montées sous `/api/admin/v1`.

mod list;
mod verify;

use axum::Router;
use axum::extract::FromRef;
use axum::routing::{get, post};
use utoipa::OpenApi;

use crate::iam::application::IamState;

#[derive(OpenApi)]
#[openapi(
    paths(list::list, verify::verify, verify::export),
    tags((name = "admin-audit", description = "Journal d'audit (ISO 27001, RGPD)"))
)]
pub struct AuditApi;

pub fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    IamState: FromRef<S>,
{
    Router::new()
        .route("/audit", get(list::list))
        .route("/audit/verify", post(verify::verify))
        .route("/audit/export", get(verify::export))
}
