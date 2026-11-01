//! Sauvegardes d'un serveur (ADR-0018) : liste, création, verrou, restauration, suppression,
//! téléchargement par ticket.

use axum::Form;
use axum::body::Body;
use axum::extract::State;
use axum::http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use kuro_protocol::api::RestoreMode;
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

use super::console::{RequestContext, denied};
use crate::http::error::{ApiError, ErrorCode, Problem, error_response};
use crate::http::extract::{Json, Path};
use crate::http::origin::CheckedOrigin;
use crate::iam::application::IamState;
use crate::iam::domain::Permission;
use crate::iam::http::AdminSession;
use crate::servers::application::backups;
use crate::servers::application::{ServersState, TICKET_TTL};
use crate::servers::infra::BackupRecord;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BackupStateDto {
    Creating,
    Ready,
    Failed,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BackupDto {
    pub id: Uuid,
    pub name: String,
    pub state: BackupStateDto,
    /// Taille de l'archive (`ready`).
    pub size_bytes: Option<i64>,
    /// SHA-256 de l'archive (`ready`).
    pub sha256: Option<String>,
    /// Jamais supprimée, ni par la rotation.
    pub locked: bool,
    /// Cause de l'échec (`failed`).
    pub failure: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub completed_at: Option<OffsetDateTime>,
}

fn dto(backup: BackupRecord) -> BackupDto {
    BackupDto {
        id: backup.id,
        name: backup.name,
        state: match backup.state.as_str() {
            "creating" => BackupStateDto::Creating,
            "ready" => BackupStateDto::Ready,
            _ => BackupStateDto::Failed,
        },
        size_bytes: backup.size_bytes,
        sha256: backup.sha256,
        locked: backup.locked,
        failure: backup.failure,
        created_at: backup.created_at,
        completed_at: backup.completed_at,
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BackupListDto {
    /// Sauvegardes prêtes gardées au plus ; 0 : sauvegardes désactivées.
    pub limit: i32,
    /// De la plus récente à la plus ancienne.
    pub backups: Vec<BackupDto>,
}

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct CreateBackupRequest {
    /// Date et heure (UTC) si absent.
    #[schema(example = "Avant la mise à jour")]
    pub name: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateBackupRequest {
    pub locked: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RestoreModeDto {
    /// Les fichiers de l'archive remplacent les leurs ; les autres restent.
    Overwrite,
    /// Volumes vidés, puis archive extraite.
    Clean,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RestoreBackupRequest {
    pub mode: RestoreModeDto,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DownloadTicketDto {
    /// À envoyer dans le corps du formulaire de téléchargement, jamais dans l'URL.
    pub ticket: String,
    pub expires_in_secs: u64,
}

/// Formulaire de téléchargement (`application/x-www-form-urlencoded`).
#[derive(Debug, Deserialize, ToSchema)]
pub struct DownloadForm {
    pub ticket: String,
}

/// Sauvegardes du serveur et leur limite. Permission : `servers.backups.read`.
#[utoipa::path(get, path = "/api/admin/v1/servers/{id}/backups", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = BackupListDto), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn list(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<Json<BackupListDto>, ApiError> {
    session
        .require(&iam, Permission::ServersBackupsRead)
        .await?;
    let (server, list) = backups::list(&state, id).await?;
    Ok(Json(BackupListDto {
        limit: server.backup_limit,
        backups: list.into_iter().map(dto).collect(),
    }))
}

/// Lance une sauvegarde, à chaud ; elle passe `ready` (ou `failed`) quand le nœud a fini. À la
/// limite, la plus ancienne non verrouillée est remplacée quand celle-ci réussit.
/// Permission : `servers.backups.create`.
#[utoipa::path(post, path = "/api/admin/v1/servers/{id}/backups", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = CreateBackupRequest,
    responses((status = 202, body = BackupDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, description = "Sauvegarde en cours, limite atteinte, sauvegardes désactivées ou serveur occupé", body = Problem),
        (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn create(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Json(request): Json<CreateBackupRequest>,
) -> Result<(StatusCode, Json<BackupDto>), ApiError> {
    session
        .require(&iam, Permission::ServersBackupsCreate)
        .await?;
    let backup = backups::create(&state, &session.context, id, request.name.as_deref()).await?;
    Ok((StatusCode::ACCEPTED, Json(dto(backup))))
}

/// Verrouille ou déverrouille. Permission : `servers.backups.create`.
#[utoipa::path(patch, path = "/api/admin/v1/servers/{id}/backups/{backup_id}", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("backup_id" = Uuid, Path)),
    request_body = UpdateBackupRequest,
    responses((status = 200, body = BackupDto), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn update(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path((id, backup_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<UpdateBackupRequest>,
) -> Result<Json<BackupDto>, ApiError> {
    session
        .require(&iam, Permission::ServersBackupsCreate)
        .await?;
    let backup =
        backups::set_locked(&state, &session.context, id, backup_id, request.locked).await?;
    Ok(Json(dto(backup)))
}

/// Restaure la sauvegarde (serveur arrêté d'abord). Accepté par le nœud ; l'état suit
/// (`restoring`, puis `offline`). Permission : `servers.backups.restore`.
#[utoipa::path(post, path = "/api/admin/v1/servers/{id}/backups/{backup_id}/restore", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("backup_id" = Uuid, Path)),
    request_body = RestoreBackupRequest,
    responses((status = 202), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 502, body = Problem)))]
pub async fn restore(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path((id, backup_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<RestoreBackupRequest>,
) -> Result<StatusCode, ApiError> {
    session
        .require_elevated(&iam, Permission::ServersBackupsRestore)
        .await?;
    let mode = match request.mode {
        RestoreModeDto::Overwrite => RestoreMode::Overwrite,
        RestoreModeDto::Clean => RestoreMode::Clean,
    };
    backups::restore(&state, &session.context, id, backup_id, mode).await?;
    Ok(StatusCode::ACCEPTED)
}

/// Supprime une sauvegarde non verrouillée. Permission : `servers.backups.delete`.
#[utoipa::path(delete, path = "/api/admin/v1/servers/{id}/backups/{backup_id}", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("backup_id" = Uuid, Path)),
    responses((status = 204), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 502, body = Problem)))]
pub async fn delete(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path((id, backup_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    session
        .require_elevated(&iam, Permission::ServersBackupsDelete)
        .await?;
    backups::delete(&state, &session.context, id, backup_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Ticket à usage unique (30 s) pour télécharger la sauvegarde.
/// Permission : `servers.backups.download`.
#[utoipa::path(post, path = "/api/admin/v1/servers/{id}/backups/{backup_id}/download/ticket", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("backup_id" = Uuid, Path)),
    responses((status = 201, body = DownloadTicketDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem)))]
pub async fn download_ticket(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path((id, backup_id)): Path<(Uuid, Uuid)>,
) -> Result<(StatusCode, Json<DownloadTicketDto>), ApiError> {
    session
        .require(&iam, Permission::ServersBackupsDownload)
        .await?;
    let ticket = backups::download_ticket(
        &state,
        &session.context,
        id,
        backup_id,
        session.principal.user_id,
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(DownloadTicketDto {
            ticket,
            expires_in_secs: TICKET_TTL.as_secs(),
        }),
    ))
}

/// Archive de la sauvegarde (`application/zstd`), pour un formulaire soumis par le navigateur :
/// le ticket voyage dans le corps, jamais dans l'URL. L'origine de la page est vérifiée.
#[utoipa::path(post, path = "/api/admin/v1/servers/{id}/backups/{backup_id}/download", tag = "admin-servers",
    params(("id" = Uuid, Path), ("backup_id" = Uuid, Path)),
    request_body(content = DownloadForm, content_type = "application/x-www-form-urlencoded"),
    responses((status = 200, description = "Archive tar.zst", content_type = "application/zstd"),
        (status = 401, body = Problem), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn download(
    State(state): State<ServersState>,
    _origin: CheckedOrigin,
    RequestContext(context): RequestContext,
    axum::extract::Path((id, backup_id)): axum::extract::Path<(Uuid, Uuid)>,
    Form(form): Form<DownloadForm>,
) -> Response {
    let redeemed = crate::servers::application::tickets::redeem_download(
        &state.pool,
        &form.ticket,
        id,
        backup_id,
    )
    .await;
    if let Err(error) = &redeemed {
        tracing::error!(%error, "failed to redeem a download ticket");
    }
    if !matches!(redeemed, Ok(Some(_))) {
        denied(
            &state,
            &context,
            json!({ "reason": "download_ticket", "server_id": id, "backup_id": backup_id }),
        )
        .await;
        return error_response(StatusCode::UNAUTHORIZED, ErrorCode::Unauthorized);
    }
    let download = match backups::download(&state, id, backup_id).await {
        Ok(download) => download,
        Err(error) => return error.into_response(),
    };
    let disposition = format!("attachment; filename=\"{}\"", download.file_name);
    let mut response = Body::from_stream(download.body).into_response();
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/zstd"));
    headers.insert(CONTENT_LENGTH, HeaderValue::from(download.size));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Ok(value) = HeaderValue::from_str(&disposition) {
        headers.insert(CONTENT_DISPOSITION, value);
    }
    response
}
