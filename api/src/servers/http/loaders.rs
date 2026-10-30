//! Loaders de la source de contenu (`fabric`, `paper`…) et leurs icônes, pour l'affichage.

use axum::extract::State;
use serde::Serialize;
use utoipa::ToSchema;

use crate::content::Loader;
use crate::http::error::{ApiError, Problem};
use crate::http::extract::Json;
use crate::iam::application::IamState;
use crate::iam::domain::Permission;
use crate::iam::http::AdminSession;
use crate::servers::application::ServersState;
use crate::servers::application::content;

#[derive(Debug, Serialize, ToSchema)]
pub struct ContentLoaderDto {
    #[schema(example = "fabric")]
    pub name: String,
    /// SVG de la source : à afficher en masque CSS (`mask-image`), jamais inséré dans la page.
    pub icon_svg: Option<String>,
}

impl From<Loader> for ContentLoaderDto {
    fn from(loader: Loader) -> Self {
        Self {
            name: loader.name,
            icon_svg: loader.icon_svg,
        }
    }
}

/// Loaders connus de la source de contenu. Permission : `servers.content.read`.
#[utoipa::path(get, path = "/api/admin/v1/content/loaders", tag = "admin-servers", security(("bearer" = [])),
    responses((status = 200, body = [ContentLoaderDto]), (status = 403, body = Problem), (status = 502, body = Problem)))]
pub async fn list(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
) -> Result<Json<Vec<ContentLoaderDto>>, ApiError> {
    session
        .require(&iam, Permission::ServersContentRead)
        .await?;
    let loaders = content::loaders(&state).await?;
    Ok(Json(loaders.into_iter().map(Into::into).collect()))
}
