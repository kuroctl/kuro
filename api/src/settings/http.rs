//! `/settings` : paramètres du projet (ADR-0019).

use axum::Router;
use axum::extract::{FromRef, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use time::OffsetDateTime;
use utoipa::{OpenApi, ToSchema};

use crate::http::error::{ApiError, Problem, error_response, internal_error};
use crate::http::extract::Json;
use crate::iam::application::IamState;
use crate::iam::domain::Permission;
use crate::iam::http::AdminSession;
use crate::oauth::application::OAuthState;
use crate::settings::application::{
    self as settings, PlatformSettings, SettingsError, SettingsPatch,
};

#[derive(OpenApi)]
#[openapi(
    paths(get_settings, update_settings),
    components(schemas(SettingsDto, UpdatePlatformSettingsRequest)),
    tags((name = "admin-settings", description = "Paramètres du projet (ADR-0019)"))
)]
pub struct SettingsApi;

pub fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    IamState: FromRef<S>,
    PgPool: FromRef<S>,
    Option<OAuthState>: FromRef<S>,
    crate::servers::application::ServersState: FromRef<S>,
{
    Router::new().route("/settings", get(get_settings).patch(update_settings))
}

impl IntoResponse for SettingsError {
    fn into_response(self) -> Response {
        match self {
            Self::Invalid(fields) => Problem::invalid_fields(fields).into_response(),
            Self::Unprocessable(code) => error_response(StatusCode::UNPROCESSABLE_ENTITY, code),
            Self::Internal(error) => internal_error(&error, "settings request failed"),
        }
    }
}

impl From<SettingsError> for ApiError {
    fn from(error: SettingsError) -> Self {
        Self::from(error.into_response())
    }
}

/// Valeurs par défaut et plafonds des ressources des serveurs ; un plafond `null` : aucun.
#[derive(Debug, Serialize, ToSchema)]
pub struct SettingsDto {
    #[schema(example = 2048)]
    pub default_memory_mb: i32,
    pub max_memory_mb: Option<i32>,
    /// 100 = un cœur.
    #[schema(example = 200)]
    pub default_cpu_percent: i32,
    pub max_cpu_percent: Option<i32>,
    /// 0 : illimité.
    #[schema(example = 10240)]
    pub default_disk_mb: i32,
    pub max_disk_mb: Option<i32>,
    #[schema(example = 3)]
    pub default_backup_limit: i32,
    pub max_backup_limit: Option<i32>,
    /// Durée de conservation du journal d'audit (30 à 3650 jours), purgé chaque jour.
    #[schema(example = 365)]
    pub audit_retention_days: i32,
    /// Serveur MCP ouvert aux agents IA (ADR-0023) ; coupé, ses jetons sont refusés.
    pub mcp_enabled: bool,
    /// Accès SFTP aux fichiers des serveurs (ADR-0047) ; coupé, toute connexion est refusée.
    pub sftp_enabled: bool,
    /// Durée de conservation d'une version d'un fichier (ADR-0049), 0 à 365 jours ; 0 coupe
    /// l'historique et efface ce qui était gardé.
    pub file_history_days: i32,
    /// Place prise au plus par l'historique d'un serveur (1 à 10 240 Mio).
    pub file_history_max_mb: i32,
    /// URL à donner au client IA (`<mcp.public_url>/mcp`) ; absente si le MCP n'est pas
    /// configuré sur ce serveur, auquel cas l'activer reste sans effet.
    pub mcp_url: Option<String>,
    /// Seuils d'alerte des stats (ADR-0043) : CPU et mémoire en % de l'allocation (50 à 100),
    /// soutenus pendant `alert_minutes` minutes (1 à 60) ; réarmés 20 points plus bas.
    #[schema(example = 90)]
    pub alert_cpu_percent: i32,
    #[schema(example = 90)]
    pub alert_memory_percent: i32,
    #[schema(example = 5)]
    pub alert_minutes: i32,
    /// Nom du projet (ADR-0045), 1 à 80 caractères ; absent : « Kuro ».
    pub project_name: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: OffsetDateTime,
}

impl SettingsDto {
    fn new(s: PlatformSettings, oauth: Option<&OAuthState>) -> Self {
        Self {
            default_memory_mb: s.default_memory_mb,
            max_memory_mb: s.max_memory_mb,
            default_cpu_percent: s.default_cpu_percent,
            max_cpu_percent: s.max_cpu_percent,
            default_disk_mb: s.default_disk_mb,
            max_disk_mb: s.max_disk_mb,
            default_backup_limit: s.default_backup_limit,
            max_backup_limit: s.max_backup_limit,
            audit_retention_days: s.audit_retention_days,
            mcp_enabled: s.mcp_enabled,
            sftp_enabled: s.sftp_enabled,
            file_history_days: s.file_history_days,
            file_history_max_mb: s.file_history_max_mb,
            mcp_url: oauth.map(OAuthState::resource),
            alert_cpu_percent: s.alert_cpu_percent,
            alert_memory_percent: s.alert_memory_percent,
            alert_minutes: s.alert_minutes,
            project_name: s.project_name,
            updated_at: s.updated_at,
        }
    }
}

/// Champs modifiés seulement ; un plafond à `null` est supprimé.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdatePlatformSettingsRequest {
    pub default_memory_mb: Option<i32>,
    #[serde(default, deserialize_with = "crate::http::patch::nullable")]
    #[schema(value_type = Option<i32>)]
    pub max_memory_mb: Option<Option<i32>>,
    pub default_cpu_percent: Option<i32>,
    #[serde(default, deserialize_with = "crate::http::patch::nullable")]
    #[schema(value_type = Option<i32>)]
    pub max_cpu_percent: Option<Option<i32>>,
    pub default_disk_mb: Option<i32>,
    #[serde(default, deserialize_with = "crate::http::patch::nullable")]
    #[schema(value_type = Option<i32>)]
    pub max_disk_mb: Option<Option<i32>>,
    pub default_backup_limit: Option<i32>,
    #[serde(default, deserialize_with = "crate::http::patch::nullable")]
    #[schema(value_type = Option<i32>)]
    pub max_backup_limit: Option<Option<i32>>,
    pub audit_retention_days: Option<i32>,
    pub mcp_enabled: Option<bool>,
    pub sftp_enabled: Option<bool>,
    pub file_history_days: Option<i32>,
    pub file_history_max_mb: Option<i32>,
    pub alert_cpu_percent: Option<i32>,
    pub alert_memory_percent: Option<i32>,
    pub alert_minutes: Option<i32>,
    /// `null` retire le nom (« Kuro »).
    #[serde(default, deserialize_with = "crate::http::patch::nullable")]
    #[schema(value_type = Option<String>)]
    pub project_name: Option<Option<String>>,
}

/// Paramètres du projet. Permission : `system.settings.read`.
#[utoipa::path(get, path = "/api/admin/v1/settings", tag = "admin-settings", security(("bearer" = [])),
    responses((status = 200, body = SettingsDto), (status = 403, body = Problem)))]
pub async fn get_settings(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    State(oauth): State<Option<OAuthState>>,
    session: AdminSession,
) -> Result<Json<SettingsDto>, ApiError> {
    session
        .require(&iam, Permission::SystemSettingsRead)
        .await?;
    Ok(Json(SettingsDto::new(
        settings::get(&pool).await?,
        oauth.as_ref(),
    )))
}

/// Modifie les paramètres ; chaque champ modifié est journalisé avec son ancienne valeur.
/// Baisser un plafond ne touche pas les serveurs existants. Permission : `system.settings.update`.
#[utoipa::path(patch, path = "/api/admin/v1/settings", tag = "admin-settings", security(("bearer" = [])),
    request_body = UpdatePlatformSettingsRequest,
    responses((status = 200, body = SettingsDto), (status = 403, body = Problem),
        (status = 422, description = "Valeur hors bornes, valeur par défaut au-delà de son plafond, ou rien à modifier", body = Problem)))]
pub async fn update_settings(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    State(oauth): State<Option<OAuthState>>,
    State(servers): State<crate::servers::application::ServersState>,
    session: AdminSession,
    Json(request): Json<UpdatePlatformSettingsRequest>,
) -> Result<Json<SettingsDto>, ApiError> {
    session
        .require(&iam, Permission::SystemSettingsUpdate)
        .await?;
    let patch = SettingsPatch {
        default_memory_mb: request.default_memory_mb,
        max_memory_mb: request.max_memory_mb,
        default_cpu_percent: request.default_cpu_percent,
        max_cpu_percent: request.max_cpu_percent,
        default_disk_mb: request.default_disk_mb,
        max_disk_mb: request.max_disk_mb,
        default_backup_limit: request.default_backup_limit,
        max_backup_limit: request.max_backup_limit,
        audit_retention_days: request.audit_retention_days,
        mcp_enabled: request.mcp_enabled,
        sftp_enabled: request.sftp_enabled,
        file_history_days: request.file_history_days,
        file_history_max_mb: request.file_history_max_mb,
        alert_cpu_percent: request.alert_cpu_percent,
        alert_memory_percent: request.alert_memory_percent,
        alert_minutes: request.alert_minutes,
        project_name: request.project_name,
    };
    let history_changed =
        request_touches_history(request.file_history_days, request.file_history_max_mb);
    let settings = settings::update(&pool, &session.context, patch).await?;
    if history_changed {
        crate::servers::application::files::send_history_policy(
            &servers,
            settings.file_history_days,
            settings.file_history_max_mb,
        );
    }
    Ok(Json(SettingsDto::new(settings, oauth.as_ref())))
}

fn request_touches_history(days: Option<i32>, max_mb: Option<i32>) -> bool {
    days.is_some() || max_mb.is_some()
}
