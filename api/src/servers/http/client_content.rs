//! Mods, plugins et contenus du Workshop des serveurs du propriétaire (ADR-0061) : mêmes cas
//! d'usage et mêmes réponses que l'espace admin (ADR-0020, ADR-0029), garde `OwnerSession` et
//! vérification du propriétaire (`404` pour le serveur d'un autre). Ni envoi de fichier, ni
//! modpack, ni valeurs des champs après l'installation : ce que le panel client ne propose pas.

use axum::extract::State;
use axum::http::StatusCode;
use uuid::Uuid;

use super::client::owned;
use super::content::{
    ContentCategoryDto, ContentDto, ContentListDto, ContentSearchDto, ContentSlotDto,
    ContentVersionDto, InstallContentRequest, RemoveParams, RemovedContentDto, SearchParams,
    SourceParams, ToggleContentRequest, dto,
};
use crate::http::error::{ApiError, Problem};
use crate::http::extract::{Json, Path, Query};
use crate::iam::http::OwnerSession;
use crate::servers::application::ServersState;
use crate::servers::application::content;

/// Emplacements de contenu du serveur (mods, plugins…) et leurs sources.
#[utoipa::path(get, path = "/api/client/v1/servers/{id}/content", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [ContentSlotDto]), (status = 401, body = Problem), (status = 404, body = Problem)))]
pub async fn slots(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<ContentSlotDto>>, ApiError> {
    owned(&state, &session, id).await?;
    let slots = content::slots(&state, id).await?;
    Ok(Json(slots.into_iter().map(Into::into).collect()))
}

/// Contenu installé d'un emplacement, avec les mises à jour disponibles.
#[utoipa::path(get, path = "/api/client/v1/servers/{id}/content/{slot}", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("slot" = String, Path)),
    responses((status = 200, body = ContentListDto), (status = 401, body = Problem), (status = 404, body = Problem)))]
pub async fn list(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path((id, slot)): Path<(Uuid, String)>,
) -> Result<Json<ContentListDto>, ApiError> {
    owned(&state, &session, id).await?;
    let installed = content::installed(&state, id, &slot).await?;
    let items = installed
        .items
        .clone()
        .into_iter()
        .map(|item| dto(item, Some(&installed)))
        .collect();
    Ok(Json(ContentListDto { items }))
}

/// Recherche chez une source, filtrée par loader et version du jeu du serveur.
#[utoipa::path(get, path = "/api/client/v1/servers/{id}/content/{slot}/search", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("slot" = String, Path), SearchParams),
    responses((status = 200, body = ContentSearchDto), (status = 401, body = Problem), (status = 404, body = Problem),
        (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn search(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path((id, slot)): Path<(Uuid, String)>,
    Query(params): Query<SearchParams>,
) -> Result<Json<ContentSearchDto>, ApiError> {
    owned(&state, &session, id).await?;
    let filters = params.filters()?;
    let page = content::search(
        &state,
        id,
        &slot,
        params.source.as_deref(),
        params.q.as_deref().unwrap_or(""),
        params.offset.unwrap_or(0),
        &filters,
    )
    .await?;
    Ok(Json(ContentSearchDto {
        total: page.total,
        hits: page.hits.into_iter().map(Into::into).collect(),
    }))
}

/// Catégories d'une source pour cet emplacement, à proposer en filtres.
#[utoipa::path(get, path = "/api/client/v1/servers/{id}/content/{slot}/categories", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("slot" = String, Path), SourceParams),
    responses((status = 200, body = [ContentCategoryDto]), (status = 401, body = Problem), (status = 404, body = Problem),
        (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn categories(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path((id, slot)): Path<(Uuid, String)>,
    Query(params): Query<SourceParams>,
) -> Result<Json<Vec<ContentCategoryDto>>, ApiError> {
    owned(&state, &session, id).await?;
    let categories = content::categories(&state, id, &slot, params.source.as_deref()).await?;
    Ok(Json(categories.into_iter().map(Into::into).collect()))
}

/// Versions compatibles d'un projet, de la plus récente à la plus ancienne.
#[utoipa::path(get, path = "/api/client/v1/servers/{id}/content/{slot}/projects/{project}/versions", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("slot" = String, Path), ("project" = String, Path), SourceParams),
    responses((status = 200, body = [ContentVersionDto]), (status = 401, body = Problem), (status = 404, body = Problem),
        (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn versions(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path((id, slot, project)): Path<(Uuid, String, String)>,
    Query(params): Query<SourceParams>,
) -> Result<Json<Vec<ContentVersionDto>>, ApiError> {
    owned(&state, &session, id).await?;
    let versions = content::versions(&state, id, &slot, params.source.as_deref(), &project).await?;
    Ok(Json(versions.into_iter().map(Into::into).collect()))
}

/// Installe une version et ses dépendances obligatoires ; pris en compte au redémarrage du jeu.
#[utoipa::path(post, path = "/api/client/v1/servers/{id}/content/{slot}", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("slot" = String, Path)),
    request_body = InstallContentRequest,
    responses((status = 202, body = [ContentDto]), (status = 401, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn install(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path((id, slot)): Path<(Uuid, String)>,
    Json(request): Json<InstallContentRequest>,
) -> Result<(StatusCode, Json<Vec<ContentDto>>), ApiError> {
    owned(&state, &session, id).await?;
    let items = content::install(
        &state,
        &session.context,
        id,
        &slot,
        request.source.as_deref(),
        &request.version_id,
        &request.values,
    )
    .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(items.into_iter().map(|i| dto(i, None)).collect()),
    ))
}

/// Met à jour vers la version compatible la plus récente.
#[utoipa::path(post, path = "/api/client/v1/servers/{id}/content/{slot}/{content_id}/update", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("slot" = String, Path), ("content_id" = Uuid, Path)),
    responses((status = 202, body = ContentDto), (status = 401, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn update(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path((id, slot, content_id)): Path<(Uuid, String, Uuid)>,
) -> Result<(StatusCode, Json<ContentDto>), ApiError> {
    owned(&state, &session, id).await?;
    let item = content::update(&state, &session.context, id, &slot, content_id).await?;
    Ok((StatusCode::ACCEPTED, Json(dto(item, None))))
}

/// Active ou désactive un contenu.
#[utoipa::path(patch, path = "/api/client/v1/servers/{id}/content/{slot}/{content_id}", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("slot" = String, Path), ("content_id" = Uuid, Path)),
    request_body = ToggleContentRequest,
    responses((status = 200, body = ContentDto), (status = 401, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 502, body = Problem)))]
pub async fn toggle(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path((id, slot, content_id)): Path<(Uuid, String, Uuid)>,
    Json(request): Json<ToggleContentRequest>,
) -> Result<Json<ContentDto>, ApiError> {
    owned(&state, &session, id).await?;
    let item = content::toggle(
        &state,
        &session.context,
        id,
        &slot,
        content_id,
        request.enabled,
    )
    .await?;
    Ok(Json(dto(item, None)))
}

/// Supprime un contenu (et, sur demande, les dépendances devenues inutiles).
#[utoipa::path(delete, path = "/api/client/v1/servers/{id}/content/{slot}/{content_id}", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("slot" = String, Path), ("content_id" = Uuid, Path), RemoveParams),
    responses((status = 200, body = RemovedContentDto), (status = 401, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 502, body = Problem)))]
pub async fn remove(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path((id, slot, content_id)): Path<(Uuid, String, Uuid)>,
    Query(params): Query<RemoveParams>,
) -> Result<Json<RemovedContentDto>, ApiError> {
    owned(&state, &session, id).await?;
    let removed = content::remove(
        &state,
        &session.context,
        id,
        &slot,
        content_id,
        params.with_orphans.unwrap_or(false),
    )
    .await?;
    Ok(Json(RemovedContentDto { removed }))
}
