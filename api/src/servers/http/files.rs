//! `/servers/{id}/files` : fichiers des volumes d'un serveur (ADR-0015).

use axum::extract::State;
use axum::http::StatusCode;
use kuro_protocol::daemon::FileKind;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::http::error::{ApiError, Problem};
use crate::http::extract::{Json, Path, Query};
use crate::iam::application::IamState;
use crate::iam::domain::Permission;
use crate::iam::http::AdminSession;
use crate::servers::application::{ServersState, files};

#[derive(Debug, Deserialize, IntoParams)]
pub struct PathQuery {
    /// `/<service>/<volume>/…` ; la racine liste les services.
    #[param(example = "/server/data")]
    #[serde(default = "root")]
    pub path: String,
}

fn root() -> String {
    "/".to_owned()
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileKindDto {
    File,
    Directory,
    /// Listé, supprimable, jamais suivi.
    Symlink,
    Other,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FileEntryDto {
    pub name: String,
    pub kind: FileKindDto,
    pub size: u64,
    /// Millisecondes Unix ; 0 pour les services et volumes.
    pub modified_ms: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FileListingDto {
    pub path: String,
    pub entries: Vec<FileEntryDto>,
    /// Le dossier compte plus d'entrées que le nœud n'en envoie : seules les premières sont là.
    pub truncated: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FileContentDto {
    pub path: String,
    pub content: String,
    pub size: u64,
    pub modified_ms: i64,
    /// À renvoyer à l'enregistrement (`if_sha256`) pour ne pas écraser une modification.
    pub sha256: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct WriteFileRequest {
    #[schema(example = "/server/data/server.properties")]
    pub path: String,
    pub content: String,
    /// Empreinte lue ; absente : création ou écrasement voulu.
    pub if_sha256: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateDirectoryRequest {
    #[schema(example = "/server/data/plugins")]
    pub path: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RenameFileRequest {
    pub from: String,
    pub to: String,
}

/// Contenu d'un dossier (dossiers d'abord). Permission : `servers.files.read`.
#[utoipa::path(get, path = "/api/admin/v1/servers/{id}/files", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), PathQuery),
    responses((status = 200, body = FileListingDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn list(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Query(query): Query<PathQuery>,
) -> Result<Json<FileListingDto>, ApiError> {
    session.require(&iam, Permission::ServersFilesRead).await?;
    let listing = files::list(&state, id, &query.path).await?;
    Ok(Json(FileListingDto {
        path: query.path,
        truncated: listing.truncated,
        entries: listing
            .entries
            .into_iter()
            .map(|e| FileEntryDto {
                name: e.name,
                kind: match e.kind {
                    FileKind::File => FileKindDto::File,
                    FileKind::Directory => FileKindDto::Directory,
                    FileKind::Symlink => FileKindDto::Symlink,
                    FileKind::Other => FileKindDto::Other,
                },
                size: e.size,
                modified_ms: e.modified_ms,
            })
            .collect(),
    }))
}

/// Contenu d'un fichier texte (512 Kio au plus). Permission : `servers.files.read`.
#[utoipa::path(get, path = "/api/admin/v1/servers/{id}/files/content", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), PathQuery),
    responses((status = 200, body = FileContentDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn read(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Query(query): Query<PathQuery>,
) -> Result<Json<FileContentDto>, ApiError> {
    session.require(&iam, Permission::ServersFilesRead).await?;
    let file = files::read(&state, id, &query.path).await?;
    Ok(Json(FileContentDto {
        path: query.path,
        content: file.content,
        size: file.size,
        modified_ms: file.modified_ms,
        sha256: file.sha256,
    }))
}

/// Crée ou remplace un fichier texte. Permission : `servers.files.write`.
#[utoipa::path(put, path = "/api/admin/v1/servers/{id}/files/content", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = WriteFileRequest,
    responses((status = 204), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn write(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Json(request): Json<WriteFileRequest>,
) -> Result<StatusCode, ApiError> {
    session.require(&iam, Permission::ServersFilesWrite).await?;
    files::write(
        &state,
        &session.context,
        id,
        &request.path,
        request.content,
        request.if_sha256,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Crée un dossier. Permission : `servers.files.write`.
#[utoipa::path(post, path = "/api/admin/v1/servers/{id}/files/directories", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = CreateDirectoryRequest,
    responses((status = 201), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn mkdir(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Json(request): Json<CreateDirectoryRequest>,
) -> Result<StatusCode, ApiError> {
    session.require(&iam, Permission::ServersFilesWrite).await?;
    files::mkdir(&state, &session.context, id, &request.path).await?;
    Ok(StatusCode::CREATED)
}

/// Renomme ou déplace, sans jamais écraser. Permission : `servers.files.write`.
#[utoipa::path(post, path = "/api/admin/v1/servers/{id}/files/rename", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = RenameFileRequest,
    responses((status = 204), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn rename(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Json(request): Json<RenameFileRequest>,
) -> Result<StatusCode, ApiError> {
    session.require(&iam, Permission::ServersFilesWrite).await?;
    files::rename(&state, &session.context, id, &request.from, &request.to).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Supprime un fichier, un lien, ou un dossier et tout son contenu.
/// Permission : `servers.files.write`.
#[utoipa::path(delete, path = "/api/admin/v1/servers/{id}/files", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), PathQuery),
    responses((status = 204), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn delete(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Query(query): Query<PathQuery>,
) -> Result<StatusCode, ApiError> {
    session.require(&iam, Permission::ServersFilesWrite).await?;
    files::delete(&state, &session.context, id, &query.path).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Taille annoncée par `Content-Length`, obligatoire pour un envoi.
pub(super) fn content_length(headers: &axum::http::HeaderMap) -> Result<u64, ApiError> {
    headers
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .ok_or_else(|| {
            crate::http::error::error_response(
                StatusCode::LENGTH_REQUIRED,
                crate::http::error::ErrorCode::InvalidBody,
            )
            .into()
        })
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UploadedFileDto {
    pub path: String,
    pub size: u64,
    /// SHA-256 calculé par l'API et vérifié par le nœud.
    pub sha256: String,
}

/// Envoie un fichier binaire (corps brut, `Content-Length` obligatoire, 256 Mio au plus), relayé
/// au nœud par morceaux ; il n'apparaît qu'une fois complet et vérifié. Remplace un fichier
/// existant. Permission : `servers.files.write`.
#[utoipa::path(put, path = "/api/admin/v1/servers/{id}/files/upload", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), PathQuery),
    request_body(content = Vec<u8>, content_type = "application/octet-stream"),
    responses((status = 201, body = UploadedFileDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 411, body = Problem), (status = 422, body = Problem),
        (status = 502, body = Problem)))]
pub async fn upload(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Query(query): Query<PathQuery>,
    headers: axum::http::HeaderMap,
    body: axum::body::Body,
) -> Result<(StatusCode, Json<UploadedFileDto>), ApiError> {
    session.require(&iam, Permission::ServersFilesWrite).await?;
    let size = content_length(&headers)?;
    let uploaded = files::upload(
        &state,
        &session.context,
        id,
        &query.path,
        size,
        body.into_data_stream(),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(UploadedFileDto {
            path: uploaded.path,
            size: uploaded.size,
            sha256: uploaded.sha256,
        }),
    ))
}
