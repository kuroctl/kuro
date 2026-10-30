//! `/regions` : emplacement physique des nœuds.

use crate::http::extract::Json;
use crate::http::extract::Path;
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error_code::{ErrorCode, FieldError};
use crate::http::error::{ApiError, Problem};
use crate::iam::application::IamState;
use crate::iam::domain::Permission;
use crate::iam::http::AdminSession;
use crate::nodes::application::{NewRegion, RegionPatch, regions};
use crate::nodes::infra::RegionRecord;

#[derive(Debug, Serialize, ToSchema)]
pub struct NodeCountsDto {
    pub total: i64,
    pub online: i64,
    pub offline: i64,
    pub pending: i64,
    pub revoked: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RegionDto {
    pub id: Uuid,
    #[schema(example = "eu-west")]
    pub slug: String,
    #[schema(example = "Europe Ouest")]
    pub name: String,
    /// Centre de la région sur la carte.
    pub latitude: f64,
    pub longitude: f64,
    pub nodes: NodeCountsDto,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
}

impl From<RegionRecord> for RegionDto {
    fn from(region: RegionRecord) -> Self {
        Self {
            id: region.id,
            slug: region.slug,
            name: region.name,
            latitude: region.latitude,
            longitude: region.longitude,
            nodes: NodeCountsDto {
                total: region.nodes_total,
                online: region.nodes_online,
                offline: region.nodes_offline,
                pending: region.nodes_pending,
                revoked: region.nodes_revoked,
            },
            created_at: region.created_at,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateRegionRequest {
    /// Identifiant fixe, `[a-z0-9-]`.
    #[schema(example = "eu-west")]
    pub slug: String,
    #[schema(example = "Europe Ouest")]
    pub name: String,
    #[schema(example = 48.8566)]
    pub latitude: f64,
    #[schema(example = 2.3522)]
    pub longitude: f64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateRegionRequest {
    pub name: Option<String>,
    /// Les deux ensemble.
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

/// Régions et nombre de nœuds par état. Permission : `nodes.read`.
#[utoipa::path(get, path = "/api/admin/v1/regions", tag = "admin-nodes", security(("bearer" = [])),
    responses((status = 200, body = [RegionDto]), (status = 403, body = Problem)))]
pub async fn list(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    session: AdminSession,
) -> Result<Json<Vec<RegionDto>>, ApiError> {
    session.require(&iam, Permission::NodesRead).await?;
    Ok(Json(
        regions::list(&pool)
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    ))
}

/// Détail d'une région. Permission : `nodes.read`.
#[utoipa::path(get, path = "/api/admin/v1/regions/{id}", tag = "admin-nodes", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = RegionDto), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn get(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<Json<RegionDto>, ApiError> {
    session.require(&iam, Permission::NodesRead).await?;
    Ok(Json(regions::get(&pool, id).await?.into()))
}

/// Crée une région. Permission : `nodes.regions.manage`.
#[utoipa::path(post, path = "/api/admin/v1/regions", tag = "admin-nodes", security(("bearer" = [])),
    request_body = CreateRegionRequest,
    responses((status = 201, body = RegionDto), (status = 403, body = Problem),
              (status = 409, body = Problem), (status = 422, body = Problem)))]
pub async fn create(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    session: AdminSession,
    Json(request): Json<CreateRegionRequest>,
) -> Result<(StatusCode, Json<RegionDto>), ApiError> {
    session
        .require(&iam, Permission::NodesRegionsManage)
        .await?;
    let region = regions::create(
        &pool,
        &session.context,
        NewRegion {
            slug: request.slug,
            name: request.name,
            latitude: request.latitude,
            longitude: request.longitude,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(region.into())))
}

/// Modifie le nom ou le centre d'une région (le slug est fixe). Permission : `nodes.regions.manage`.
#[utoipa::path(patch, path = "/api/admin/v1/regions/{id}", tag = "admin-nodes", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = UpdateRegionRequest,
    responses((status = 200, body = RegionDto), (status = 403, body = Problem),
              (status = 404, body = Problem), (status = 422, body = Problem)))]
pub async fn update(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Json(request): Json<UpdateRegionRequest>,
) -> Result<Json<RegionDto>, ApiError> {
    session
        .require(&iam, Permission::NodesRegionsManage)
        .await?;
    let coordinates = match (request.latitude, request.longitude) {
        (Some(latitude), Some(longitude)) => Some((latitude, longitude)),
        (None, None) => None,
        _ => {
            return Err(
                crate::nodes::application::NodesError::Invalid(FieldError::new(
                    "coordinates",
                    ErrorCode::InvalidCoordinates,
                ))
                .into(),
            );
        }
    };
    let patch = RegionPatch {
        name: request.name,
        coordinates,
    };
    Ok(Json(
        regions::update(&pool, &session.context, id, patch)
            .await?
            .into(),
    ))
}

/// Supprime une région vide. Permission : `nodes.regions.manage`.
#[utoipa::path(delete, path = "/api/admin/v1/regions/{id}", tag = "admin-nodes", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 204), (status = 403, body = Problem), (status = 404, body = Problem),
              (status = 409, description = "La région contient des nœuds", body = Problem)))]
pub async fn delete(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    session
        .require(&iam, Permission::NodesRegionsManage)
        .await?;
    regions::delete(&pool, &session.context, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
