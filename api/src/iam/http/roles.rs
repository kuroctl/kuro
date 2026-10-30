//! `/roles` : rôles dynamiques.

use crate::http::extract::Json;
use crate::http::extract::Path;
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

use super::AdminSession;
use crate::http::error::{ApiError, Problem};
use crate::iam::application::{IamState, NewRole, RolePatch, roles};
use crate::iam::domain::Permission;
use crate::iam::infra::RoleRecord;

#[derive(Debug, Serialize, ToSchema)]
pub struct RoleDto {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    /// Couleur du badge, `#rrggbb`.
    #[schema(example = "#6366f1")]
    pub color: String,
    /// Rôle livré par Kuro (`super-admin`) : ni modifiable ni supprimable.
    pub is_system: bool,
    /// Clés de permission ; `*` pour toutes.
    pub permissions: Vec<String>,
    pub members: i64,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
}

impl From<RoleRecord> for RoleDto {
    fn from(role: RoleRecord) -> Self {
        let mut permissions = role.permissions;
        permissions.sort();
        Self {
            id: role.id,
            name: role.name,
            description: role.description,
            color: role.color,
            is_system: role.is_system,
            permissions,
            members: role.members,
            created_at: role.created_at,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateRoleRequest {
    #[schema(example = "support-n1")]
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// `#rrggbb`, ardoise neutre par défaut.
    pub color: Option<String>,
    #[schema(example = json!(["iam.users.read", "audit.read"]))]
    pub permissions: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateRoleRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub color: Option<String>,
    /// Remplace l'ensemble des permissions.
    pub permissions: Option<Vec<String>>,
}

/// Liste des rôles. Permission : `iam.roles.read`.
#[utoipa::path(get, path = "/api/admin/v1/roles", tag = "admin-iam", security(("bearer" = [])),
    responses((status = 200, body = [RoleDto]), (status = 403, body = Problem)))]
pub async fn list(
    State(state): State<IamState>,
    session: AdminSession,
) -> Result<Json<Vec<RoleDto>>, ApiError> {
    session.require(&state, Permission::IamRolesRead).await?;
    Ok(Json(
        roles::list(&state)
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    ))
}

/// Détail d'un rôle. Permission : `iam.roles.read`.
#[utoipa::path(get, path = "/api/admin/v1/roles/{id}", tag = "admin-iam", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = RoleDto), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn get(
    State(state): State<IamState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<Json<RoleDto>, ApiError> {
    session.require(&state, Permission::IamRolesRead).await?;
    Ok(Json(roles::get(&state, id).await?.into()))
}

/// Crée un rôle. Permission : `iam.roles.manage` (et détenir chaque permission accordée).
#[utoipa::path(post, path = "/api/admin/v1/roles", tag = "admin-iam", security(("bearer" = [])),
    request_body = CreateRoleRequest,
    responses((status = 201, body = RoleDto), (status = 403, body = Problem),
              (status = 409, body = Problem), (status = 422, body = Problem)))]
pub async fn create(
    State(state): State<IamState>,
    session: AdminSession,
    Json(request): Json<CreateRoleRequest>,
) -> Result<(StatusCode, Json<RoleDto>), ApiError> {
    let authority = session.require(&state, Permission::IamRolesManage).await?;
    let role = roles::create(
        &state,
        &authority,
        &session.context,
        NewRole {
            name: request.name,
            description: request.description,
            color: request.color,
            permissions: request.permissions,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(role.into())))
}

/// Modifie un rôle. Permission : `iam.roles.manage`.
#[utoipa::path(patch, path = "/api/admin/v1/roles/{id}", tag = "admin-iam", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = UpdateRoleRequest,
    responses((status = 200, body = RoleDto), (status = 403, body = Problem), (status = 404, body = Problem),
              (status = 409, body = Problem), (status = 422, body = Problem)))]
pub async fn update(
    State(state): State<IamState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Json(request): Json<UpdateRoleRequest>,
) -> Result<Json<RoleDto>, ApiError> {
    let authority = session.require(&state, Permission::IamRolesManage).await?;
    let patch = RolePatch {
        name: request.name,
        description: request.description,
        color: request.color,
        permissions: request.permissions,
    };
    Ok(Json(
        roles::update(&state, &authority, &session.context, id, patch)
            .await?
            .into(),
    ))
}

/// Supprime un rôle (retiré des comptes qui le portaient). Permission : `iam.roles.manage`.
#[utoipa::path(delete, path = "/api/admin/v1/roles/{id}", tag = "admin-iam", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 204), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn delete(
    State(state): State<IamState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let authority = session.require(&state, Permission::IamRolesManage).await?;
    roles::delete(&state, &authority, &session.context, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
