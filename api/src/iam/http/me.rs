//! `/me` : compte connecté.

use crate::http::extract::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::AdminSession;
use super::users::AdminUserDto;
use crate::http::error::{ApiError, Problem};
use crate::iam::application::{IamState, users};

#[derive(Debug, Serialize, ToSchema)]
pub struct ProfileDto {
    pub user: AdminUserDto,
    /// Permissions effectives (catalogue complet pour un super-admin).
    pub permissions: Vec<&'static str>,
    /// Vrai pour un super-admin : toutes les permissions, y compris futures.
    pub all_permissions: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

/// Profil et permissions effectives. Aucune permission requise.
#[utoipa::path(get, path = "/api/admin/v1/me", tag = "admin-iam", security(("bearer" = [])),
    responses((status = 200, body = ProfileDto), (status = 401, body = Problem)))]
pub async fn get(
    State(state): State<IamState>,
    session: AdminSession,
) -> Result<Json<ProfileDto>, ApiError> {
    let profile = users::profile(&state, &session.principal).await?;
    Ok(Json(ProfileDto {
        user: profile.user.into(),
        permissions: profile
            .permissions
            .into_iter()
            .map(|permission| permission.key())
            .collect(),
        all_permissions: profile.all_permissions,
    }))
}

/// Change son mot de passe ; les autres sessions sont fermées. Aucune permission requise.
#[utoipa::path(put, path = "/api/admin/v1/me/password", tag = "admin-iam", security(("bearer" = [])),
    request_body = ChangePasswordRequest,
    responses((status = 204), (status = 401, body = Problem), (status = 422, body = Problem)))]
pub async fn change_password(
    State(state): State<IamState>,
    session: AdminSession,
    Json(request): Json<ChangePasswordRequest>,
) -> Result<StatusCode, ApiError> {
    users::change_own_password(
        &state,
        &session.principal,
        &session.context,
        &request.current_password,
        &request.new_password,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
