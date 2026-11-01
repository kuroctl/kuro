//! Routes SFTP (ADR-0047) : clés SSH du compte connecté (`/me/ssh-keys`) et coordonnées de
//! connexion d'un serveur (`/servers/{id}/sftp`).

use axum::Router;
use axum::extract::{FromRef, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

use super::application::{self, Connection, SftpError, SftpState};
use super::infra::KeyRecord;
use crate::error_code::ErrorCode;
use crate::http::error::{ApiError, Problem, error_response, internal_error};
use crate::http::extract::Json;
use crate::iam::application::IamState;
use crate::iam::domain::Permission;
use crate::iam::http::AdminSession;

#[derive(OpenApi)]
#[openapi(
    paths(list_keys, add_key, remove_key, get_connection),
    components(schemas(SshKeyDto, AddSshKeyRequest, SftpConnectionDto)),
    tags((name = "admin-sftp", description = "Accès SFTP (ADR-0047)"))
)]
pub struct SftpApi;

pub fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    IamState: FromRef<S>,
    SftpState: FromRef<S>,
{
    Router::new()
        .route("/me/ssh-keys", get(list_keys).post(add_key))
        .route("/me/ssh-keys/{key_id}", delete(remove_key))
        .route("/servers/{id}/sftp", get(get_connection))
}

impl IntoResponse for SftpError {
    fn into_response(self) -> Response {
        match self {
            Self::Invalid(field) => Problem::invalid(field).into_response(),
            Self::Conflict(code) => error_response(StatusCode::CONFLICT, code),
            Self::NotFound(code) => error_response(StatusCode::NOT_FOUND, code),
            Self::Internal(error) => internal_error(&error, "sftp request failed"),
        }
    }
}

impl From<SftpError> for ApiError {
    fn from(error: SftpError) -> Self {
        Self::from(error.into_response())
    }
}

/// Clé SSH d'un compte, sans la clé elle-même.
#[derive(Debug, Serialize, ToSchema)]
pub struct SshKeyDto {
    pub id: Uuid,
    pub name: String,
    /// `ssh-ed25519`, `ecdsa-sha2-nistp256`…
    pub algorithm: String,
    /// `SHA256:…`, comme l'affiche `ssh-keygen -lf`.
    pub fingerprint: String,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub last_used_at: Option<OffsetDateTime>,
}

impl From<KeyRecord> for SshKeyDto {
    fn from(record: KeyRecord) -> Self {
        Self {
            id: record.id,
            name: record.name,
            algorithm: record.algorithm,
            fingerprint: record.fingerprint,
            created_at: record.created_at,
            last_used_at: record.last_used_at,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AddSshKeyRequest {
    pub name: String,
    /// Ligne OpenSSH : `ssh-ed25519 AAAA… commentaire`.
    pub public_key: String,
}

/// Coordonnées SFTP d'un serveur pour le compte connecté.
#[derive(Debug, Serialize, ToSchema)]
pub struct SftpConnectionDto {
    /// SFTP ouvert dans les paramètres du projet.
    pub enabled: bool,
    /// Le nœud sert le SFTP (port et clé d'hôte annoncés).
    pub available: bool,
    /// Nom DNS du nœud, sinon son adresse publique saisie, sinon l'adresse vue sur son lien (hors
    /// boucle locale) ; absent sinon.
    pub host: Option<String>,
    pub port: Option<u16>,
    /// `<serveur>.<compte>` : propre au compte connecté.
    pub username: String,
    /// Empreinte de la clé d'hôte du nœud, à comparer à celle que montre le client.
    pub host_key_fingerprint: Option<String>,
    pub node_name: String,
}

impl From<Connection> for SftpConnectionDto {
    fn from(connection: Connection) -> Self {
        Self {
            enabled: connection.enabled,
            available: connection.available,
            host: connection.host,
            port: connection.port,
            username: connection.username,
            host_key_fingerprint: connection.host_key_fingerprint,
            node_name: connection.node_name,
        }
    }
}

/// Clés SSH du compte connecté. Aucune permission requise.
#[utoipa::path(get, path = "/api/admin/v1/me/ssh-keys", tag = "admin-sftp", security(("bearer" = [])),
    responses((status = 200, body = [SshKeyDto]), (status = 401, body = Problem)))]
pub async fn list_keys(
    State(sftp): State<SftpState>,
    session: AdminSession,
) -> Result<Json<Vec<SshKeyDto>>, ApiError> {
    let keys = application::keys(&sftp, &session.principal).await?;
    Ok(Json(keys.into_iter().map(SshKeyDto::from).collect()))
}

/// Ajoute une clé SSH au compte connecté (ed25519 ou ECDSA). Aucune permission requise :
/// ouvrir une session SFTP exige en plus `servers.files.sftp`.
#[utoipa::path(post, path = "/api/admin/v1/me/ssh-keys", tag = "admin-sftp", security(("bearer" = [])),
    request_body = AddSshKeyRequest,
    responses((status = 201, body = SshKeyDto), (status = 401, body = Problem),
        (status = 409, body = Problem), (status = 422, body = Problem)))]
pub async fn add_key(
    State(sftp): State<SftpState>,
    session: AdminSession,
    Json(request): Json<AddSshKeyRequest>,
) -> Result<(StatusCode, Json<SshKeyDto>), ApiError> {
    let key = application::add_key(
        &sftp,
        &session.context,
        &session.principal,
        &request.name,
        &request.public_key,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(key.into())))
}

/// Retire une clé du compte connecté ; les sessions qui l'utilisent ferment au bail suivant.
#[utoipa::path(delete, path = "/api/admin/v1/me/ssh-keys/{key_id}", tag = "admin-sftp", security(("bearer" = [])),
    params(("key_id" = Uuid, Path)),
    responses((status = 204), (status = 401, body = Problem), (status = 404, body = Problem)))]
pub async fn remove_key(
    State(sftp): State<SftpState>,
    session: AdminSession,
    Path(key_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    application::remove_key(&sftp, &session.context, &session.principal, key_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Coordonnées SFTP du serveur pour le compte connecté. Permission : `servers.files.sftp`.
#[utoipa::path(get, path = "/api/admin/v1/servers/{id}/sftp", tag = "admin-sftp", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = SftpConnectionDto), (status = 403, body = Problem),
        (status = 404, body = Problem)))]
pub async fn get_connection(
    State(iam): State<IamState>,
    State(sftp): State<SftpState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<Json<SftpConnectionDto>, ApiError> {
    session.require(&iam, Permission::ServersFilesSftp).await?;
    let connection = application::connection(&sftp, &session.principal, id)
        .await?
        .ok_or(SftpError::NotFound(ErrorCode::NotFound))?;
    Ok(Json(connection.into()))
}
