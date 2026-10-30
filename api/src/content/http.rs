//! Sources de contenu (ADR-0029) : clés d'API dans les paramètres (`system.content_sources.manage`).
//! Une clé est vérifiée auprès de la source à l'enregistrement, et n'est jamais renvoyée.

use axum::Router;
use axum::extract::{FromRef, State};
use axum::http::StatusCode;
use axum::routing::{get, put};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::{OpenApi, ToSchema};

use super::provider::ProviderError;
use super::settings::{self, ContentSourcesState, SourceKeyError};
use crate::error_code::{ErrorCode, FieldError};
use crate::http::error::{ApiError, Problem, internal_error};
use crate::http::extract::{Json, Path};
use crate::iam::application::IamState;
use crate::iam::domain::Permission;
use crate::iam::http::AdminSession;

#[derive(OpenApi)]
#[openapi(
    paths(list_sources, put_source_key, delete_source_key),
    tags((name = "admin-content-sources", description = "Sources de contenu à clé d'API (ADR-0029)"))
)]
pub struct ContentSourcesApi;

pub fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    IamState: FromRef<S>,
    ContentSourcesState: FromRef<S>,
{
    Router::new()
        .route("/settings/content-sources", get(list_sources))
        .route(
            "/settings/content-sources/{provider}",
            put(put_source_key).delete(delete_source_key),
        )
}

impl From<SourceKeyError> for ApiError {
    fn from(error: SourceKeyError) -> Self {
        let (status, code) = match &error {
            SourceKeyError::UnknownSource => {
                (StatusCode::NOT_FOUND, ErrorCode::UnknownContentSource)
            }
            SourceKeyError::NotConfigured => {
                (StatusCode::NOT_FOUND, ErrorCode::ContentSourceNotConfigured)
            }
            SourceKeyError::KeyRequired => {
                return Problem::invalid(FieldError::new(
                    "api_key",
                    ErrorCode::ContentSourceKeyRequired,
                ))
                .into();
            }
            SourceKeyError::Provider(ProviderError::Unauthorized) => {
                return Problem::invalid(FieldError::new(
                    "api_key",
                    ErrorCode::ContentSourceUnauthorized,
                ))
                .into();
            }
            SourceKeyError::Provider(_) => {
                tracing::warn!(%error, "content source check failed");
                (StatusCode::BAD_GATEWAY, ErrorCode::ProviderUnavailable)
            }
            SourceKeyError::Database(_) | SourceKeyError::Internal(_) => {
                return internal_error(&error, "content source request failed").into();
            }
        };
        Problem::new(status, code).into()
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ContentSourceDto {
    /// `curseforge`, `steam-workshop`.
    pub provider: String,
    /// Une clé est enregistrée (jamais renvoyée).
    pub configured: bool,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub updated_at: Option<OffsetDateTime>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SourceKeyRequest {
    /// Clé d'API de la source (console.curseforge.com pour CurseForge,
    /// steamcommunity.com/dev/apikey pour Steam Workshop).
    pub api_key: String,
}

/// Sources à clé d'API et leur état. Permission : `system.content_sources.manage`.
#[utoipa::path(get, path = "/api/admin/v1/settings/content-sources", tag = "admin-content-sources",
    security(("bearer" = [])),
    responses((status = 200, body = Vec<ContentSourceDto>), (status = 403, body = Problem)))]
pub async fn list_sources(
    State(iam): State<IamState>,
    State(state): State<ContentSourcesState>,
    session: AdminSession,
) -> Result<Json<Vec<ContentSourceDto>>, ApiError> {
    session
        .require(&iam, Permission::SystemContentSourcesManage)
        .await?;
    Ok(Json(
        settings::status(&state)
            .await?
            .into_iter()
            .map(|source| ContentSourceDto {
                provider: source.provider.to_owned(),
                configured: source.updated_at.is_some(),
                updated_at: source.updated_at,
            })
            .collect(),
    ))
}

/// Enregistre la clé d'une source, vérifiée auprès d'elle d'abord. Permission :
/// `system.content_sources.manage`.
#[utoipa::path(put, path = "/api/admin/v1/settings/content-sources/{provider}", tag = "admin-content-sources",
    security(("bearer" = [])), params(("provider" = String, Path)), request_body = SourceKeyRequest,
    responses((status = 204), (status = 403, body = Problem), (status = 404, body = Problem),
              (status = 422, description = "Clé absente ou refusée par la source", body = Problem),
              (status = 502, description = "Source injoignable", body = Problem)))]
pub async fn put_source_key(
    State(iam): State<IamState>,
    State(state): State<ContentSourcesState>,
    session: AdminSession,
    Path(provider): Path<String>,
    Json(request): Json<SourceKeyRequest>,
) -> Result<StatusCode, ApiError> {
    session
        .require(&iam, Permission::SystemContentSourcesManage)
        .await?;
    settings::set_key(&state, &session.context, &provider, &request.api_key).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Retire la clé d'une source. Permission : `system.content_sources.manage`.
#[utoipa::path(delete, path = "/api/admin/v1/settings/content-sources/{provider}", tag = "admin-content-sources",
    security(("bearer" = [])), params(("provider" = String, Path)),
    responses((status = 204), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn delete_source_key(
    State(iam): State<IamState>,
    State(state): State<ContentSourcesState>,
    session: AdminSession,
    Path(provider): Path<String>,
) -> Result<StatusCode, ApiError> {
    session
        .require(&iam, Permission::SystemContentSourcesManage)
        .await?;
    settings::remove_key(&state, &session.context, &provider).await?;
    Ok(StatusCode::NO_CONTENT)
}
