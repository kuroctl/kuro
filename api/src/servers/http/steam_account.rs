//! Compte Steam de l'installation (ADR-0033), dans les paramètres
//! (`system.steam_accounts.manage`). Le mot de passe n'est jamais renvoyé.

use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;

use crate::http::error::{ApiError, Problem};
use crate::http::extract::Json;
use crate::iam::application::IamState;
use crate::iam::domain::Permission;
use crate::iam::http::AdminSession;
use crate::servers::application::ServersState;
use crate::servers::application::steam_account;

#[derive(Debug, Serialize, ToSchema)]
pub struct SteamAccountDto {
    /// Un compte est réglé (son mot de passe n'est jamais renvoyé).
    pub configured: bool,
    /// Identifiant du compte, affiché pour s'y retrouver.
    pub username: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub updated_at: Option<OffsetDateTime>,
}

#[derive(Deserialize, ToSchema)]
pub struct SteamAccountRequest {
    /// Compte dédié aux serveurs, sans Steam Guard.
    pub username: String,
    pub password: String,
}

/// Compte Steam de l'installation, sans son mot de passe. Permission :
/// `system.steam_accounts.manage`.
#[utoipa::path(get, path = "/api/admin/v1/settings/steam-account", tag = "admin-servers",
    security(("bearer" = [])),
    responses((status = 200, body = SteamAccountDto), (status = 403, body = Problem)))]
pub async fn get(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
) -> Result<Json<SteamAccountDto>, ApiError> {
    session
        .require(&iam, Permission::SystemSteamAccountsManage)
        .await?;
    let status = steam_account::status(&state).await?;
    Ok(Json(SteamAccountDto {
        configured: status.is_some(),
        username: status.as_ref().map(|s| s.username.clone()),
        updated_at: status.map(|s| s.updated_at),
    }))
}

/// Enregistre ou remplace le compte. Permission : `system.steam_accounts.manage`.
#[utoipa::path(put, path = "/api/admin/v1/settings/steam-account", tag = "admin-servers",
    security(("bearer" = [])), request_body = SteamAccountRequest,
    responses((status = 204), (status = 403, body = Problem),
              (status = 422, description = "Identifiant ou mot de passe invalide", body = Problem)))]
pub async fn put(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Json(request): Json<SteamAccountRequest>,
) -> Result<StatusCode, ApiError> {
    session
        .require_elevated(&iam, Permission::SystemSteamAccountsManage)
        .await?;
    steam_account::set(
        &state,
        &session.context,
        &request.username,
        &request.password,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Retire le compte. Permission : `system.steam_accounts.manage`.
#[utoipa::path(delete, path = "/api/admin/v1/settings/steam-account", tag = "admin-servers",
    security(("bearer" = [])),
    responses((status = 204), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn delete(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
) -> Result<StatusCode, ApiError> {
    session
        .require_elevated(&iam, Permission::SystemSteamAccountsManage)
        .await?;
    steam_account::remove(&state, &session.context).await?;
    Ok(StatusCode::NO_CONTENT)
}
