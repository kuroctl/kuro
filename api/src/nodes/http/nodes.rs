//! `/nodes` : lecture et emplacement. Création et tokens : `crate::enrollment::http::admin`.

use std::sync::Arc;

use crate::http::extract::Json;
use crate::http::extract::{Path, Query};
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use time::OffsetDateTime;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::http::error::{ApiError, Problem};
use crate::iam::application::IamState;
use crate::iam::domain::Permission;
use crate::iam::http::AdminSession;
use crate::nodes::application::{NodePatch, inventory, lifecycle, placement};
use crate::nodes::domain::NodeLinks;
use crate::nodes::infra::{NodeRecord, RegionFilter};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
pub struct CoordinatesDto {
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RegionRefDto {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
}

/// Caractéristiques annoncées par le daemon à son dernier `hello`.
#[derive(Debug, Serialize, ToSchema)]
pub struct HostDto {
    pub daemon_version: String,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub cpu_cores: Option<i32>,
    pub memory_bytes: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct NodeDto {
    pub id: Uuid,
    /// Identifiant technique, inscrit dans le certificat : non modifiable.
    pub name: String,
    /// Libellé modifiable ; afficher `name` s'il est absent.
    pub display_name: Option<String>,
    #[schema(example = "online")]
    pub status: String,
    pub region: Option<RegionRefDto>,
    pub location: Option<String>,
    /// Coordonnées propres au nœud.
    pub coordinates: Option<CoordinatesDto>,
    /// Position sur la carte : coordonnées propres, sinon centre de la région.
    pub map_position: Option<CoordinatesDto>,
    /// Absent tant que le daemon ne s'est jamais connecté.
    pub host: Option<HostDto>,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub last_heartbeat_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
    /// Nœud de test : accepte les essais du Studio (serveurs à durée limitée).
    pub trials: bool,
    /// Adresses publiques, publiées sous le nom du nœud (ADR-0026).
    pub public_ipv4: Option<String>,
    pub public_ipv6: Option<String>,
}

impl From<NodeRecord> for NodeDto {
    fn from(node: NodeRecord) -> Self {
        let coordinates = pair(node.latitude, node.longitude);
        let region_center = pair(node.region_latitude, node.region_longitude);
        let region = match (node.region_id, node.region_slug, node.region_name) {
            (Some(id), Some(slug), Some(name)) => Some(RegionRefDto { id, slug, name }),
            _ => None,
        };
        Self {
            id: node.id,
            name: node.name,
            display_name: node.display_name,
            status: node.status,
            region,
            location: node.location,
            coordinates,
            map_position: coordinates.or(region_center),
            host: node.daemon_version.map(|daemon_version| HostDto {
                daemon_version,
                os: node.os,
                arch: node.arch,
                cpu_cores: node.cpu_cores,
                memory_bytes: node.memory_bytes,
            }),
            last_heartbeat_at: node.last_heartbeat_at,
            created_at: node.created_at,
            trials: node.trials,
            public_ipv4: node.public_ipv4.clone(),
            public_ipv6: node.public_ipv6.clone(),
        }
    }
}

fn pair(latitude: Option<f64>, longitude: Option<f64>) -> Option<CoordinatesDto> {
    Some(CoordinatesDto {
        latitude: latitude?,
        longitude: longitude?,
    })
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct NodesQuery {
    /// Nœuds d'une région.
    pub region_id: Option<Uuid>,
    /// `true` : uniquement les nœuds sans région.
    #[serde(default)]
    pub unassigned: bool,
}

/// PATCH : champ absent = inchangé, `null` = effacé.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateNodeRequest {
    #[serde(default, deserialize_with = "crate::http::patch::nullable")]
    #[schema(value_type = Option<Uuid>)]
    pub region_id: Option<Option<Uuid>>,
    #[serde(default, deserialize_with = "crate::http::patch::nullable")]
    #[schema(value_type = Option<String>)]
    pub display_name: Option<Option<String>>,
    #[serde(default, deserialize_with = "crate::http::patch::nullable")]
    #[schema(value_type = Option<String>)]
    pub location: Option<Option<String>>,
    #[serde(default, deserialize_with = "crate::http::patch::nullable")]
    #[schema(value_type = Option<CoordinatesDto>)]
    pub coordinates: Option<Option<CoordinatesDto>>,
    /// Nœud de test : accepte les essais du Studio.
    pub trials: Option<bool>,
}

/// Nœuds, filtrables par région. Permission : `nodes.read`.
#[utoipa::path(get, path = "/api/admin/v1/nodes", tag = "admin-nodes", security(("bearer" = [])),
    params(NodesQuery),
    responses((status = 200, body = [NodeDto]), (status = 403, body = Problem)))]
pub async fn list(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    session: AdminSession,
    Query(query): Query<NodesQuery>,
) -> Result<Json<Vec<NodeDto>>, ApiError> {
    session.require(&iam, Permission::NodesRead).await?;
    let filter = match (query.region_id, query.unassigned) {
        (Some(region), _) => RegionFilter::Region(region),
        (None, true) => RegionFilter::Unassigned,
        (None, false) => RegionFilter::All,
    };
    Ok(Json(
        inventory::list(&pool, filter)
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    ))
}

/// Détail d'un nœud. Permission : `nodes.read`.
#[utoipa::path(get, path = "/api/admin/v1/nodes/{id}", tag = "admin-nodes", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = NodeDto), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn get(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<Json<NodeDto>, ApiError> {
    session.require(&iam, Permission::NodesRead).await?;
    Ok(Json(inventory::get(&pool, id).await?.into()))
}

/// Libellé, région, localisation, coordonnées, nœud de test. Permission : `nodes.update`.
#[utoipa::path(patch, path = "/api/admin/v1/nodes/{id}", tag = "admin-nodes", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = UpdateNodeRequest,
    responses((status = 200, body = NodeDto), (status = 403, body = Problem),
              (status = 404, body = Problem), (status = 422, body = Problem)))]
pub async fn update(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Json(request): Json<UpdateNodeRequest>,
) -> Result<Json<NodeDto>, ApiError> {
    session.require(&iam, Permission::NodesUpdate).await?;
    let patch = NodePatch {
        region_id: request.region_id,
        display_name: request.display_name,
        location: request.location,
        coordinates: request
            .coordinates
            .map(|value| value.map(|c| (c.latitude, c.longitude))),
        trials: request.trials,
    };
    Ok(Json(
        placement::update(&pool, &session.context, id, patch)
            .await?
            .into(),
    ))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RevokeNodeRequest {
    /// Motif, conservé dans le journal d'audit.
    pub reason: Option<String>,
}

/// Révoque le certificat du nœud et coupe sa session. Réenrôlement : nouveau token.
/// Permission : `nodes.revoke`.
#[utoipa::path(post, path = "/api/admin/v1/nodes/{id}/revoke", tag = "admin-nodes", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = RevokeNodeRequest,
    responses((status = 200, body = NodeDto), (status = 403, body = Problem), (status = 404, body = Problem),
              (status = 409, description = "Déjà révoqué, ou pas encore enrôlé", body = Problem)))]
pub async fn revoke(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    State(links): State<Arc<dyn NodeLinks>>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Json(request): Json<RevokeNodeRequest>,
) -> Result<Json<NodeDto>, ApiError> {
    session.require(&iam, Permission::NodesRevoke).await?;
    let node =
        lifecycle::revoke(&pool, &session.context, links.as_ref(), id, request.reason).await?;
    Ok(Json(node.into()))
}

/// Supprime un nœud en attente, hors ligne ou révoqué ; ses tokens partent avec lui et son
/// certificat reste refusé. Permission : `nodes.delete`.
#[utoipa::path(delete, path = "/api/admin/v1/nodes/{id}", tag = "admin-nodes", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 204), (status = 403, body = Problem), (status = 404, body = Problem),
              (status = 409, description = "Nœud en ligne (`node_online`), ou serveurs encore assignés (`node_has_servers`)", body = Problem)))]
pub async fn delete(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    State(links): State<Arc<dyn NodeLinks>>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    session.require(&iam, Permission::NodesDelete).await?;
    lifecycle::delete(&pool, &session.context, links.as_ref(), id).await?;
    Ok(StatusCode::NO_CONTENT)
}
