//! `/settings/mail` (ADR-0046) : réglages SMTP du projet, sans jamais rendre le mot de passe, et
//! envoi d'un e-mail de test à l'admin connecté.

use axum::Router;
use axum::extract::{FromRef, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::{OpenApi, ToSchema};

use super::application::{MailError, MailInput, MailSettingsView, MailState};
use crate::error_code::ErrorCode;
use crate::http::error::{ApiError, Problem, error_response, internal_error};
use crate::http::extract::Json;
use crate::iam::application::{IamState, users};
use crate::iam::domain::Permission;
use crate::iam::http::AdminSession;

#[derive(OpenApi)]
#[openapi(
    paths(get_mail, put_mail, delete_mail, test_mail),
    components(schemas(MailSettingsDto, MailSettingsRequest, MailTestDto)),
    tags((name = "admin-mail", description = "Envoi d'e-mails (ADR-0046)"))
)]
pub struct MailApi;

pub fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    IamState: FromRef<S>,
    MailState: FromRef<S>,
{
    Router::new()
        .route(
            "/settings/mail",
            get(get_mail).put(put_mail).delete(delete_mail),
        )
        .route("/settings/mail/test", post(test_mail))
}

impl IntoResponse for MailError {
    fn into_response(self) -> Response {
        match self {
            Self::Invalid(field) => Problem::invalid(field).into_response(),
            Self::NotConfigured => {
                error_response(StatusCode::NOT_FOUND, ErrorCode::MailNotConfigured)
            }
            Self::Send(error) => Problem::new(StatusCode::BAD_GATEWAY, ErrorCode::MailSendFailed)
                .param("error", error)
                .into_response(),
            Self::Internal(error) => internal_error(&error, "mail request failed"),
        }
    }
}

impl From<MailError> for ApiError {
    fn from(error: MailError) -> Self {
        Self::from(error.into_response())
    }
}

/// Réglages SMTP, sans le mot de passe.
#[derive(Debug, Serialize, ToSchema)]
pub struct MailSettingsDto {
    /// Un serveur est réglé : l'e-mail est proposé pour les canaux et les invitations.
    pub configured: bool,
    /// `security = "none"` accepté par cette API (`mail.allow_insecure`, dev seulement).
    pub insecure_allowed: bool,
    pub host: Option<String>,
    pub port: Option<u16>,
    /// `starttls`, `tls`, ou `none` quand l'API le permet.
    pub security: Option<String>,
    pub username: Option<String>,
    pub has_password: bool,
    pub from_address: Option<String>,
    /// Nom d'expéditeur ; absent : le nom du projet.
    pub from_name: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub updated_at: Option<OffsetDateTime>,
}

impl MailSettingsDto {
    fn new(view: Option<MailSettingsView>, insecure_allowed: bool) -> Self {
        match view {
            Some(view) => Self {
                configured: true,
                insecure_allowed,
                host: Some(view.host),
                port: Some(view.port),
                security: Some(view.security.as_str().to_owned()),
                username: view.username,
                has_password: view.has_password,
                from_address: Some(view.from_address),
                from_name: view.from_name,
                updated_at: Some(view.updated_at),
            },
            None => Self {
                configured: false,
                insecure_allowed,
                host: None,
                port: None,
                security: None,
                username: None,
                has_password: false,
                from_address: None,
                from_name: None,
                updated_at: None,
            },
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MailSettingsRequest {
    pub host: String,
    pub port: u16,
    /// `starttls` (587) ou `tls` (465) ; `none` seulement si l'API le permet (dev).
    pub security: String,
    #[serde(default)]
    pub username: Option<String>,
    /// Absent : le mot de passe enregistré est gardé ; vide : retiré.
    #[serde(default)]
    pub password: Option<String>,
    pub from_address: String,
    #[serde(default)]
    pub from_name: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MailTestDto {
    pub ok: bool,
    /// Adresse qui a reçu le test : celle de l'admin connecté.
    pub to: String,
}

/// Réglages SMTP du projet. Permission : `system.settings.read`.
#[utoipa::path(get, path = "/api/admin/v1/settings/mail", tag = "admin-mail", security(("bearer" = [])),
    responses((status = 200, body = MailSettingsDto), (status = 403, body = Problem)))]
pub async fn get_mail(
    State(iam): State<IamState>,
    State(mail): State<MailState>,
    session: AdminSession,
) -> Result<Json<MailSettingsDto>, ApiError> {
    session
        .require(&iam, Permission::SystemSettingsRead)
        .await?;
    Ok(Json(MailSettingsDto::new(
        mail.get().await?,
        mail.allow_insecure,
    )))
}

/// Enregistre ou remplace les réglages ; le mot de passe est chiffré. Permission :
/// `system.settings.update`.
#[utoipa::path(put, path = "/api/admin/v1/settings/mail", tag = "admin-mail", security(("bearer" = [])),
    request_body = MailSettingsRequest,
    responses((status = 200, body = MailSettingsDto), (status = 403, body = Problem), (status = 422, body = Problem)))]
pub async fn put_mail(
    State(iam): State<IamState>,
    State(mail): State<MailState>,
    session: AdminSession,
    Json(request): Json<MailSettingsRequest>,
) -> Result<Json<MailSettingsDto>, ApiError> {
    session
        .require(&iam, Permission::SystemSettingsUpdate)
        .await?;
    let view = mail
        .set(
            &session.context,
            MailInput {
                host: request.host,
                port: request.port,
                security: request.security,
                username: request.username,
                password: request.password,
                from_address: request.from_address,
                from_name: request.from_name,
            },
        )
        .await?;
    Ok(Json(MailSettingsDto::new(Some(view), mail.allow_insecure)))
}

/// Retire les réglages : plus d'e-mail jusqu'au prochain réglage. Permission :
/// `system.settings.update`.
#[utoipa::path(delete, path = "/api/admin/v1/settings/mail", tag = "admin-mail", security(("bearer" = [])),
    responses((status = 204), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn delete_mail(
    State(iam): State<IamState>,
    State(mail): State<MailState>,
    session: AdminSession,
) -> Result<StatusCode, ApiError> {
    session
        .require(&iam, Permission::SystemSettingsUpdate)
        .await?;
    mail.remove(&session.context).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Envoie un e-mail de test à l'adresse de l'admin connecté. Permission :
/// `system.settings.update`.
#[utoipa::path(post, path = "/api/admin/v1/settings/mail/test", tag = "admin-mail", security(("bearer" = [])),
    responses((status = 200, body = MailTestDto), (status = 403, body = Problem),
        (status = 404, description = "Aucun serveur SMTP réglé", body = Problem),
        (status = 502, description = "Le serveur SMTP a refusé l'envoi (`error`)", body = Problem)))]
pub async fn test_mail(
    State(iam): State<IamState>,
    State(mail): State<MailState>,
    session: AdminSession,
) -> Result<Json<MailTestDto>, ApiError> {
    session
        .require(&iam, Permission::SystemSettingsUpdate)
        .await?;
    let profile = users::profile(&iam, &session.principal).await?;
    mail.test(&session.context, &profile.user.email).await?;
    Ok(Json(MailTestDto {
        ok: true,
        to: profile.user.email,
    }))
}
