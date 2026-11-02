//! Fichiers des serveurs du propriétaire (ADR-0055) : mêmes cas d'usage et mêmes réponses que
//! `/api/admin/v1/servers/{id}/files…`, garde `OwnerSession` et vérification du propriétaire
//! (`404` pour le serveur d'un autre). Un serveur suspendu refuse ses fichiers, comme pour
//! l'admin.

use axum::extract::State;
use axum::http::StatusCode;
use uuid::Uuid;

use super::client::owned;
use super::files::{
    CreateDirectoryRequest, DeletedFilesDto, FileContentDto, FileHistoryDto, FileListingDto,
    PathQuery, RenameFileRequest, RestoreFileRequest, UploadedFileDto, WriteFileRequest,
    content_length, version_dto,
};
use crate::http::error::{ApiError, Problem};
use crate::http::extract::{Json, Path, Query};
use crate::iam::http::OwnerSession;
use crate::servers::application::{ServersState, files};

/// Contenu d'un dossier (dossiers d'abord).
#[utoipa::path(get, path = "/api/client/v1/servers/{id}/files", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), PathQuery),
    responses((status = 200, body = FileListingDto), (status = 401, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn list(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path(id): Path<Uuid>,
    Query(query): Query<PathQuery>,
) -> Result<Json<FileListingDto>, ApiError> {
    owned(&state, &session, id).await?;
    let listing = files::list(&state, id, &query.path).await?;
    Ok(Json(FileListingDto::of(query.path, listing)))
}

/// Contenu d'un fichier texte (512 Kio au plus).
#[utoipa::path(get, path = "/api/client/v1/servers/{id}/files/content", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), PathQuery),
    responses((status = 200, body = FileContentDto), (status = 401, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn read(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path(id): Path<Uuid>,
    Query(query): Query<PathQuery>,
) -> Result<Json<FileContentDto>, ApiError> {
    owned(&state, &session, id).await?;
    let file = files::read(&state, id, &query.path).await?;
    Ok(Json(FileContentDto::of(query.path, file)))
}

/// Crée ou remplace un fichier texte (`if_sha256` : ne pas écraser une modification).
#[utoipa::path(put, path = "/api/client/v1/servers/{id}/files/content", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = WriteFileRequest,
    responses((status = 204), (status = 401, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn write(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path(id): Path<Uuid>,
    Json(request): Json<WriteFileRequest>,
) -> Result<StatusCode, ApiError> {
    owned(&state, &session, id).await?;
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

/// Crée un dossier.
#[utoipa::path(post, path = "/api/client/v1/servers/{id}/files/directories", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = CreateDirectoryRequest,
    responses((status = 201), (status = 401, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn mkdir(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path(id): Path<Uuid>,
    Json(request): Json<CreateDirectoryRequest>,
) -> Result<StatusCode, ApiError> {
    owned(&state, &session, id).await?;
    files::mkdir(&state, &session.context, id, &request.path).await?;
    Ok(StatusCode::CREATED)
}

/// Renomme ou déplace, sans jamais écraser.
#[utoipa::path(post, path = "/api/client/v1/servers/{id}/files/rename", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = RenameFileRequest,
    responses((status = 204), (status = 401, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn rename(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path(id): Path<Uuid>,
    Json(request): Json<RenameFileRequest>,
) -> Result<StatusCode, ApiError> {
    owned(&state, &session, id).await?;
    files::rename(&state, &session.context, id, &request.from, &request.to).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Supprime un fichier, un lien, ou un dossier et tout son contenu.
#[utoipa::path(delete, path = "/api/client/v1/servers/{id}/files", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), PathQuery),
    responses((status = 204), (status = 401, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn delete(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path(id): Path<Uuid>,
    Query(query): Query<PathQuery>,
) -> Result<StatusCode, ApiError> {
    owned(&state, &session, id).await?;
    files::delete(&state, &session.context, id, &query.path).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Envoie un fichier binaire (corps brut, `Content-Length` obligatoire, 256 Mio au plus).
#[utoipa::path(put, path = "/api/client/v1/servers/{id}/files/upload", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), PathQuery),
    request_body(content = Vec<u8>, content_type = "application/octet-stream"),
    responses((status = 201, body = UploadedFileDto), (status = 401, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 411, body = Problem), (status = 422, body = Problem),
        (status = 502, body = Problem)))]
pub async fn upload(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path(id): Path<Uuid>,
    Query(query): Query<PathQuery>,
    headers: axum::http::HeaderMap,
    body: axum::body::Body,
) -> Result<(StatusCode, Json<UploadedFileDto>), ApiError> {
    owned(&state, &session, id).await?;
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

/// Versions gardées d'un fichier, de la plus récente à la plus ancienne.
#[utoipa::path(get, path = "/api/client/v1/servers/{id}/files/history", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), PathQuery),
    responses((status = 200, body = FileHistoryDto), (status = 401, body = Problem), (status = 404, body = Problem),
        (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn history(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path(id): Path<Uuid>,
    Query(query): Query<PathQuery>,
) -> Result<Json<FileHistoryDto>, ApiError> {
    owned(&state, &session, id).await?;
    let history = files::history(&state, id, &query.path).await?;
    Ok(Json(FileHistoryDto {
        path: query.path,
        versions: history.versions.into_iter().map(version_dto).collect(),
    }))
}

/// Contenu texte d'une version.
#[utoipa::path(get, path = "/api/client/v1/servers/{id}/files/history/{version}", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("version" = Uuid, Path), PathQuery),
    responses((status = 200, body = FileContentDto), (status = 401, body = Problem), (status = 404, body = Problem),
        (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn version(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path((id, version)): Path<(Uuid, Uuid)>,
    Query(query): Query<PathQuery>,
) -> Result<Json<FileContentDto>, ApiError> {
    owned(&state, &session, id).await?;
    let file = files::version(&state, id, &query.path, version).await?;
    Ok(Json(FileContentDto::of(query.path, file)))
}

/// Remet une version à la place du fichier ; la version remplacée est gardée.
#[utoipa::path(post, path = "/api/client/v1/servers/{id}/files/history/{version}/restore", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("version" = Uuid, Path)),
    request_body = RestoreFileRequest,
    responses((status = 204), (status = 401, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn restore(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path((id, version)): Path<(Uuid, Uuid)>,
    Json(request): Json<RestoreFileRequest>,
) -> Result<StatusCode, ApiError> {
    owned(&state, &session, id).await?;
    files::restore(&state, &session.context, id, &request.path, version).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Fichiers supprimés d'un dossier dont une version est gardée.
#[utoipa::path(get, path = "/api/client/v1/servers/{id}/files/deleted", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), PathQuery),
    responses((status = 200, body = DeletedFilesDto), (status = 401, body = Problem), (status = 404, body = Problem),
        (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn deleted(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path(id): Path<Uuid>,
    Query(query): Query<PathQuery>,
) -> Result<Json<DeletedFilesDto>, ApiError> {
    owned(&state, &session, id).await?;
    let deleted = files::deleted(&state, id, &query.path).await?;
    Ok(Json(DeletedFilesDto::of(query.path, deleted)))
}
