//! Routes HTTP de l'espace admin pour le domaine `nodes`, montées sous `/api/admin/v1`.
//! La création d'un nœud et ses tokens relèvent de [`crate::enrollment::http`].

mod allocations;
mod error;
pub mod nodes;
mod regions;

use axum::Router;
use axum::extract::FromRef;
use axum::routing::{get, post};
use std::sync::Arc;

use sqlx::PgPool;
use utoipa::OpenApi;

use crate::iam::application::IamState;
use crate::nodes::domain::NodeLinks;

pub use nodes::NodeDto;

#[derive(OpenApi)]
#[openapi(
    paths(
        regions::list, regions::get, regions::create, regions::update, regions::delete,
        nodes::list, nodes::get, nodes::update, nodes::revoke, nodes::delete,
        allocations::list, allocations::create, allocations::delete,
    ),
    tags((name = "admin-nodes", description = "Nœuds et régions (emplacement physique)"))
)]
pub struct NodesApi;

pub fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    IamState: FromRef<S>,
    PgPool: FromRef<S>,
    Arc<dyn NodeLinks>: FromRef<S>,
{
    Router::new()
        .route("/regions", get(regions::list).post(regions::create))
        .route(
            "/regions/{id}",
            get(regions::get)
                .patch(regions::update)
                .delete(regions::delete),
        )
        .route("/nodes", get(nodes::list))
        .route(
            "/nodes/{id}",
            get(nodes::get).patch(nodes::update).delete(nodes::delete),
        )
        .route("/nodes/{id}/revoke", post(nodes::revoke))
        .route(
            "/nodes/{id}/allocations",
            get(allocations::list).post(allocations::create),
        )
        .route(
            "/nodes/{id}/allocations/{allocation_id}",
            axum::routing::delete(allocations::delete),
        )
}
