//! `/users` : comptes admin.

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
use crate::iam::application::{AdminUser, IamState, NewAdminUser, UserPatch, invitations, users};
use crate::iam::domain::Permission;
use crate::mail::MailState;

#[derive(Debug, Serialize, ToSchema)]
pub struct RoleRefDto {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AdminUserDto {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub totp_enabled: bool,
    pub disabled: bool,
    /// Mot de passe choisi par un autre admin : remplacé à la prochaine connexion.
    pub password_change_required: bool,
    /// Une invitation par e-mail attend d'être acceptée (ADR-0046).
    pub invitation_pending: bool,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub last_login_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
    pub roles: Vec<RoleRefDto>,
}

impl From<AdminUser> for AdminUserDto {
    fn from(user: AdminUser) -> Self {
        Self {
            id: user.id,
            email: user.email,
            display_name: user.display_name,
            totp_enabled: user.totp_enabled,
            disabled: user.disabled,
            password_change_required: user.password_change_required,
            invitation_pending: user.invitation_pending,
            last_login_at: user.last_login_at,
            created_at: user.created_at,
            roles: user
                .roles
                .into_iter()
                .map(|(id, name)| RoleRefDto { id, name })
                .collect(),
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateUserRequest {
    pub email: String,
    pub display_name: String,
    /// Mot de passe temporaire, 12 caractères minimum ; absent avec `invite`.
    #[serde(default)]
    pub password: Option<String>,
    /// Envoie une invitation par e-mail (ADR-0046) au lieu d'un mot de passe temporaire ; exige
    /// un SMTP réglé.
    #[serde(default)]
    pub invite: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateUserRequest {
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub disabled: Option<bool>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetRolesRequest {
    pub role_ids: Vec<Uuid>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ResetPasswordRequest {
    pub password: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RevokedSessionsResponse {
    pub revoked: u64,
}

/// Liste des comptes. Permission : `iam.users.read`.
#[utoipa::path(get, path = "/api/admin/v1/users", tag = "admin-iam", security(("bearer" = [])),
    responses((status = 200, body = [AdminUserDto]), (status = 403, body = Problem)))]
pub async fn list(
    State(state): State<IamState>,
    session: AdminSession,
) -> Result<Json<Vec<AdminUserDto>>, ApiError> {
    session.require(&state, Permission::IamUsersRead).await?;
    let users = users::list(&state).await?;
    Ok(Json(users.into_iter().map(Into::into).collect()))
}

/// Détail d'un compte. Permission : `iam.users.read`.
#[utoipa::path(get, path = "/api/admin/v1/users/{id}", tag = "admin-iam", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = AdminUserDto), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn get(
    State(state): State<IamState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<Json<AdminUserDto>, ApiError> {
    session.require(&state, Permission::IamUsersRead).await?;
    Ok(Json(users::get(&state, id).await?.into()))
}

/// Crée un compte, sans rôle, avec un mot de passe temporaire : à remplacer à la première
/// connexion. Permission : `iam.users.create`.
#[utoipa::path(post, path = "/api/admin/v1/users", tag = "admin-iam", security(("bearer" = [])),
    request_body = CreateUserRequest,
    responses((status = 201, body = AdminUserDto), (status = 403, body = Problem),
              (status = 409, description = "E-mail déjà utilisé", body = Problem),
              (status = 422, body = Problem)))]
pub async fn create(
    State(state): State<IamState>,
    State(mail): State<MailState>,
    session: AdminSession,
    Json(request): Json<CreateUserRequest>,
) -> Result<(StatusCode, Json<AdminUserDto>), ApiError> {
    let authority = session.require(&state, Permission::IamUsersCreate).await?;
    // Invité : un mot de passe aléatoire jeté, que seul le lien remplace.
    let password = match (request.invite, request.password) {
        (true, _) => crate::auth::opaque::generate("")
            .map_err(|e| crate::iam::application::IamError::Internal(e.to_string()))?,
        (false, Some(password)) => password,
        (false, None) => {
            return Err(crate::iam::application::IamError::Invalid(
                crate::error_code::FieldError::new(
                    "password",
                    crate::error_code::ErrorCode::WeakPassword,
                ),
            )
            .into());
        }
    };
    if request.invite
        && !mail
            .configured()
            .await
            .map_err(|e| crate::iam::application::IamError::Internal(e.to_string()))?
    {
        return Err(crate::iam::application::IamError::Unprocessable(
            crate::error_code::ErrorCode::MailNotConfigured,
        )
        .into());
    }
    let user = users::create(
        &state,
        &authority,
        &session.context,
        NewAdminUser {
            email: request.email,
            display_name: request.display_name,
            password,
        },
    )
    .await?;
    if request.invite {
        invitations::invite(&state, &mail, &authority, &session.context, user.id).await?;
    }
    Ok((
        StatusCode::CREATED,
        Json(users::get(&state, user.id).await?.into()),
    ))
}

/// Envoie (ou renvoie) une invitation par e-mail : nouveau lien, l'ancien révoqué. Exige un
/// SMTP réglé. Permission : `iam.users.credentials`.
#[utoipa::path(post, path = "/api/admin/v1/users/{id}/invite", tag = "admin-iam", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 204), (status = 403, body = Problem), (status = 404, body = Problem),
              (status = 422, description = "SMTP non réglé ou envoi refusé", body = Problem)))]
pub async fn invite(
    State(state): State<IamState>,
    State(mail): State<MailState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let authority = session
        .require(&state, Permission::IamUsersCredentials)
        .await?;
    invitations::invite(&state, &mail, &authority, &session.context, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Modifie e-mail, nom, état. Permission : `iam.users.update`.
#[utoipa::path(patch, path = "/api/admin/v1/users/{id}", tag = "admin-iam", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = UpdateUserRequest,
    responses((status = 200, body = AdminUserDto), (status = 403, body = Problem),
              (status = 404, body = Problem), (status = 409, body = Problem), (status = 422, body = Problem)))]
pub async fn update(
    State(state): State<IamState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Json(request): Json<UpdateUserRequest>,
) -> Result<Json<AdminUserDto>, ApiError> {
    let authority = session.require(&state, Permission::IamUsersUpdate).await?;
    let patch = UserPatch {
        email: request.email,
        display_name: request.display_name,
        disabled: request.disabled,
    };
    Ok(Json(
        users::update(&state, &authority, &session.context, id, patch)
            .await?
            .into(),
    ))
}

/// Supprime un compte. Permission : `iam.users.delete`.
#[utoipa::path(delete, path = "/api/admin/v1/users/{id}", tag = "admin-iam", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 204), (status = 403, body = Problem), (status = 404, body = Problem), (status = 409, body = Problem)))]
pub async fn delete(
    State(state): State<IamState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let authority = session.require(&state, Permission::IamUsersDelete).await?;
    users::delete(&state, &authority, &session.context, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Remplace les rôles du compte. Permission : `iam.roles.assign`.
#[utoipa::path(put, path = "/api/admin/v1/users/{id}/roles", tag = "admin-iam", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = SetRolesRequest,
    responses((status = 200, body = AdminUserDto), (status = 403, body = Problem),
              (status = 404, body = Problem), (status = 409, body = Problem), (status = 422, body = Problem)))]
pub async fn set_roles(
    State(state): State<IamState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Json(request): Json<SetRolesRequest>,
) -> Result<Json<AdminUserDto>, ApiError> {
    let authority = session.require(&state, Permission::IamRolesAssign).await?;
    let user =
        users::set_roles(&state, &authority, &session.context, id, &request.role_ids).await?;
    Ok(Json(user.into()))
}

/// Impose un nouveau mot de passe et ferme les sessions. Permission : `iam.users.credentials`.
#[utoipa::path(put, path = "/api/admin/v1/users/{id}/password", tag = "admin-iam", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = ResetPasswordRequest,
    responses((status = 204), (status = 403, body = Problem), (status = 404, body = Problem), (status = 422, body = Problem)))]
pub async fn reset_password(
    State(state): State<IamState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Json(request): Json<ResetPasswordRequest>,
) -> Result<StatusCode, ApiError> {
    let authority = session
        .require(&state, Permission::IamUsersCredentials)
        .await?;
    users::reset_password(&state, &authority, &session.context, id, &request.password).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Réinitialise le TOTP (à reconfigurer à la connexion). Permission : `iam.users.credentials`.
#[utoipa::path(delete, path = "/api/admin/v1/users/{id}/totp", tag = "admin-iam", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 204), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn reset_totp(
    State(state): State<IamState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let authority = session
        .require(&state, Permission::IamUsersCredentials)
        .await?;
    users::reset_totp(&state, &authority, &session.context, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Ferme toutes les sessions du compte. Permission : `iam.users.credentials`.
#[utoipa::path(delete, path = "/api/admin/v1/users/{id}/sessions", tag = "admin-iam", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = RevokedSessionsResponse), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn revoke_sessions(
    State(state): State<IamState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<Json<RevokedSessionsResponse>, ApiError> {
    let authority = session
        .require(&state, Permission::IamUsersCredentials)
        .await?;
    let revoked = users::revoke_sessions(&state, &authority, &session.context, id).await?;
    Ok(Json(RevokedSessionsResponse { revoked }))
}
