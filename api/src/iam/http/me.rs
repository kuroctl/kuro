//! `/me` : compte connecté (profil, mot de passe, TOTP et codes de secours).

use crate::http::extract::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::AdminSession;
use super::auth::TotpSetupResponse;
use super::users::AdminUserDto;
use crate::http::error::{ApiError, Problem};
use crate::iam::application::{self, IamState, users};

#[derive(Debug, Serialize, ToSchema)]
pub struct ProfileDto {
    pub user: AdminUserDto,
    /// Permissions effectives (catalogue complet pour un super-admin).
    pub permissions: Vec<&'static str>,
    /// Vrai pour un super-admin : toutes les permissions, y compris futures.
    pub all_permissions: bool,
    /// La plateforme exige le TOTP (`auth.require_totp`) : il ne peut pas être retiré.
    pub totp_required: bool,
    /// Codes de secours du TOTP encore utilisables.
    pub recovery_codes_remaining: i64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

/// Ré-authentification avant de toucher au TOTP.
#[derive(Deserialize, ToSchema)]
pub struct TotpReauthRequest {
    pub password: String,
    /// Code TOTP ou code de secours ; exigé si le TOTP est actif.
    pub code: Option<String>,
}

/// Jamais le mot de passe ni le code, même dans un journal de débogage.
impl std::fmt::Debug for TotpReauthRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TotpReauthRequest(<redacted>)")
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TotpConfirmRequest {
    /// Code donné par l'application d'authentification pour le nouveau secret.
    #[schema(example = "123456")]
    pub code: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RecoveryCodesResponse {
    /// Codes de secours à usage unique (`xxxx-xxxx-xxxx`), lisibles cette seule fois.
    pub recovery_codes: Vec<String>,
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
        totp_required: profile.totp_required,
        recovery_codes_remaining: profile.recovery_codes_remaining,
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

/// Commence la configuration de son TOTP : premier TOTP, ou remplacement (code actuel exigé).
/// Le secret attend sa confirmation par `/me/totp/confirm`. Aucune permission requise.
#[utoipa::path(post, path = "/api/admin/v1/me/totp/setup", tag = "admin-iam", security(("bearer" = [])),
    request_body = TotpReauthRequest,
    responses((status = 200, body = TotpSetupResponse), (status = 401, body = Problem),
        (status = 422, description = "Mot de passe ou code refusé, ou code manquant", body = Problem)))]
pub async fn start_totp_setup(
    State(state): State<IamState>,
    session: AdminSession,
    Json(request): Json<TotpReauthRequest>,
) -> Result<Json<TotpSetupResponse>, ApiError> {
    let setup = application::start_own_totp_setup(
        &state,
        &session.principal,
        &session.context,
        &request.password,
        request.code.as_deref(),
    )
    .await?;
    Ok(Json(TotpSetupResponse {
        secret: setup.secret,
        otpauth_uri: setup.otpauth_uri,
    }))
}

/// Active le TOTP en attente par un premier code et délivre de nouveaux codes de secours. Un
/// remplacement ferme les autres sessions. Aucune permission requise.
#[utoipa::path(post, path = "/api/admin/v1/me/totp/confirm", tag = "admin-iam", security(("bearer" = [])),
    request_body = TotpConfirmRequest,
    responses((status = 200, body = RecoveryCodesResponse), (status = 401, body = Problem),
        (status = 409, description = "Aucune configuration en cours", body = Problem),
        (status = 422, description = "Code refusé", body = Problem)))]
pub async fn confirm_totp(
    State(state): State<IamState>,
    session: AdminSession,
    Json(request): Json<TotpConfirmRequest>,
) -> Result<Json<RecoveryCodesResponse>, ApiError> {
    let recovery_codes =
        application::confirm_own_totp(&state, &session.principal, &session.context, &request.code)
            .await?;
    Ok(Json(RecoveryCodesResponse { recovery_codes }))
}

/// Remplace ses codes de secours. Aucune permission requise.
#[utoipa::path(post, path = "/api/admin/v1/me/totp/recovery-codes", tag = "admin-iam", security(("bearer" = [])),
    request_body = TotpReauthRequest,
    responses((status = 200, body = RecoveryCodesResponse), (status = 401, body = Problem),
        (status = 409, description = "TOTP non configuré", body = Problem),
        (status = 422, description = "Mot de passe ou code refusé, ou code manquant", body = Problem)))]
pub async fn regenerate_recovery_codes(
    State(state): State<IamState>,
    session: AdminSession,
    Json(request): Json<TotpReauthRequest>,
) -> Result<Json<RecoveryCodesResponse>, ApiError> {
    let recovery_codes = application::regenerate_recovery_codes(
        &state,
        &session.principal,
        &session.context,
        &request.password,
        request.code.as_deref(),
    )
    .await?;
    Ok(Json(RecoveryCodesResponse { recovery_codes }))
}

/// Retire son TOTP et ses codes de secours, si la plateforme ne l'exige pas. Aucune permission
/// requise.
#[utoipa::path(delete, path = "/api/admin/v1/me/totp", tag = "admin-iam", security(("bearer" = [])),
    request_body = TotpReauthRequest,
    responses((status = 204), (status = 401, body = Problem),
        (status = 403, description = "TOTP exigé par la plateforme", body = Problem),
        (status = 409, description = "TOTP non configuré", body = Problem),
        (status = 422, description = "Mot de passe ou code refusé, ou code manquant", body = Problem)))]
pub async fn disable_totp(
    State(state): State<IamState>,
    session: AdminSession,
    Json(request): Json<TotpReauthRequest>,
) -> Result<StatusCode, ApiError> {
    application::disable_own_totp(
        &state,
        &session.principal,
        &session.context,
        &request.password,
        request.code.as_deref(),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Élévation de la session (ADR-0050) : mot de passe si le compte n'a pas de TOTP, code TOTP ou
/// code de secours sinon.
#[derive(Deserialize, ToSchema)]
pub struct ElevateRequest {
    pub password: Option<String>,
    /// Code TOTP ou code de secours ; exigé si le TOTP est actif.
    pub code: Option<String>,
}

/// Jamais le mot de passe ni le code, même dans un journal de débogage.
impl std::fmt::Debug for ElevateRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ElevateRequest(<redacted>)")
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ElevationResponse {
    /// Secondes pendant lesquelles les actions protégées passent sans nouvelle preuve.
    pub expires_in: u64,
}

/// Élève la session pour les actions protégées (`403 elevation_required`). Aucune permission
/// requise.
#[utoipa::path(post, path = "/api/admin/v1/me/elevate", tag = "admin-iam", security(("bearer" = [])),
    request_body = ElevateRequest,
    responses((status = 200, body = ElevationResponse), (status = 401, body = Problem),
        (status = 422, description = "Mot de passe ou code refusé, ou code manquant", body = Problem)))]
pub async fn elevate(
    State(state): State<IamState>,
    session: AdminSession,
    Json(request): Json<ElevateRequest>,
) -> Result<Json<ElevationResponse>, ApiError> {
    let ttl = application::elevation::elevate(
        &state,
        &session.principal,
        &session.context,
        request.password.as_deref(),
        request.code.as_deref(),
    )
    .await?;
    Ok(Json(ElevationResponse {
        expires_in: ttl.as_secs(),
    }))
}
