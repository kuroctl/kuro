//! `/nodes/{id}/allocations` : ports des nœuds.

use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::http::error::{ApiError, Problem};
use crate::http::extract::{Json, Path};
use crate::iam::application::IamState;
use crate::iam::domain::Permission;
use crate::iam::http::AdminSession;
use crate::nodes::application::allocations;
use crate::nodes::infra::AllocationRecord;

#[derive(Debug, Serialize, ToSchema)]
pub struct AllocationDto {
    pub id: Uuid,
    #[schema(example = "0.0.0.0")]
    pub ip: String,
    #[schema(example = 25565)]
    pub port: i32,
    /// Serveur qui utilise ce port, s'il est assigné.
    pub server_id: Option<Uuid>,
    pub server_name: Option<String>,
    /// Port du prefab servi (`game`, `http`…).
    pub port_name: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
}

impl From<AllocationRecord> for AllocationDto {
    fn from(a: AllocationRecord) -> Self {
        Self {
            id: a.id,
            ip: a.ip,
            port: a.port,
            server_id: a.server_id,
            server_name: a.server_name,
            port_name: a.port_name,
            created_at: a.created_at,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateAllocationsRequest {
    /// Adresse d'écoute sur l'hôte ; `0.0.0.0` par défaut.
    #[schema(example = "0.0.0.0")]
    pub ip: Option<String>,
    /// Port, plage ou liste : `25565`, `25565-25600`, `25565,25570-25575` (1024 au moins,
    /// 1000 ports au plus). Les ports existants sont ignorés.
    #[schema(example = "25565-25600")]
    pub ports: String,
}

/// Ports du nœud. Permission : `nodes.allocations.read`.
#[utoipa::path(get, path = "/api/admin/v1/nodes/{id}/allocations", tag = "admin-nodes", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [AllocationDto]), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn list(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<AllocationDto>>, ApiError> {
    session
        .require(&iam, Permission::NodesAllocationsRead)
        .await?;
    Ok(Json(
        allocations::list(&pool, id)
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    ))
}

/// Ajoute une plage de ports au nœud et retourne tous ses ports.
/// Permission : `nodes.allocations.manage`.
#[utoipa::path(post, path = "/api/admin/v1/nodes/{id}/allocations", tag = "admin-nodes", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = CreateAllocationsRequest,
    responses((status = 201, body = [AllocationDto]), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 422, description = "Adresse ou plage invalide", body = Problem)))]
pub async fn create(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Json(request): Json<CreateAllocationsRequest>,
) -> Result<(StatusCode, Json<Vec<AllocationDto>>), ApiError> {
    session
        .require(&iam, Permission::NodesAllocationsManage)
        .await?;
    let all = allocations::create(
        &pool,
        &session.context,
        id,
        request.ip.as_deref(),
        &request.ports,
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(all.into_iter().map(Into::into).collect()),
    ))
}

/// Supprime un port libre. Permission : `nodes.allocations.manage`.
#[utoipa::path(delete, path = "/api/admin/v1/nodes/{id}/allocations/{allocation_id}", tag = "admin-nodes", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("allocation_id" = Uuid, Path)),
    responses((status = 204), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, description = "Port assigné à un serveur", body = Problem)))]
pub async fn delete(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    session: AdminSession,
    Path((id, allocation_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    session
        .require(&iam, Permission::NodesAllocationsManage)
        .await?;
    allocations::delete(&pool, &session.context, id, allocation_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
