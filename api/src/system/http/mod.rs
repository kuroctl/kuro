//! Routes HTTP du domaine `system`.

mod health;
mod ready;

use axum::Router;
use axum::routing::get;
use utoipa::OpenApi;

use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(
    paths(health::health, ready::ready),
    tags((name = "system", description = "État de l'instance"))
)]
pub struct SystemApi;

/// Routes montées sous `/api`, hors des espaces admin et client.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/health", get(health::health))
        .route("/ready", get(ready::ready))
}
