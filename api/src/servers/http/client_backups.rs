//! Sauvegardes et tâches planifiées des serveurs du propriétaire (ADR-0056) : mêmes cas d'usage et
//! mêmes réponses que l'espace admin, garde `OwnerSession` et vérification du propriétaire (`404`
//! pour le serveur d'un autre). Une tâche planifiée ne fait que ce que l'espace client permet
//! (`Authority::Owner` : commandes, alimentation, sauvegardes).

use axum::extract::State;
use axum::http::StatusCode;
use uuid::Uuid;

use super::backups::{
    BackupDto, BackupListDto, CreateBackupRequest, DownloadTicketDto, RestoreBackupRequest,
    RestoreModeDto, UpdateBackupRequest, dto as backup_dto,
};
use super::client::owned;
use super::schedules::{
    PreviewDto, PreviewRequest, RunDto, ScheduleDto, ScheduleRequest, dto as schedule_dto, run_dto,
};
use crate::http::error::{ApiError, Problem};
use crate::http::extract::{Json, Path};
use crate::iam::domain::Authority;
use crate::iam::http::OwnerSession;
use crate::servers::application::schedules::ScheduleInput;
use crate::servers::application::tickets::TicketHolder;
use crate::servers::application::{ServersState, TICKET_TTL, backups, schedules};
use kuro_protocol::api::RestoreMode;

// Sauvegardes.

/// Sauvegardes du serveur et leur limite (celle de la formule).
#[utoipa::path(get, path = "/api/client/v1/servers/{id}/backups", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = BackupListDto), (status = 401, body = Problem), (status = 404, body = Problem)))]
pub async fn list_backups(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path(id): Path<Uuid>,
) -> Result<Json<BackupListDto>, ApiError> {
    owned(&state, &session, id).await?;
    let (server, list) = backups::list(&state, id).await?;
    Ok(Json(BackupListDto {
        limit: server.backup_limit,
        backups: list.into_iter().map(backup_dto).collect(),
    }))
}

/// Lance une sauvegarde, à chaud ; elle passe `ready` (ou `failed`) quand le nœud a fini.
#[utoipa::path(post, path = "/api/client/v1/servers/{id}/backups", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = CreateBackupRequest,
    responses((status = 202, body = BackupDto), (status = 401, body = Problem), (status = 404, body = Problem),
        (status = 409, description = "Sauvegarde en cours, limite atteinte, sauvegardes désactivées ou serveur occupé", body = Problem),
        (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn create_backup(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path(id): Path<Uuid>,
    Json(request): Json<CreateBackupRequest>,
) -> Result<(StatusCode, Json<BackupDto>), ApiError> {
    owned(&state, &session, id).await?;
    let backup = backups::create(&state, &session.context, id, request.name.as_deref()).await?;
    Ok((StatusCode::ACCEPTED, Json(backup_dto(backup))))
}

/// Verrouille ou déverrouille (une sauvegarde verrouillée n'est jamais remplacée).
#[utoipa::path(patch, path = "/api/client/v1/servers/{id}/backups/{backup_id}", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("backup_id" = Uuid, Path)),
    request_body = UpdateBackupRequest,
    responses((status = 200, body = BackupDto), (status = 401, body = Problem), (status = 404, body = Problem)))]
pub async fn update_backup(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path((id, backup_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<UpdateBackupRequest>,
) -> Result<Json<BackupDto>, ApiError> {
    owned(&state, &session, id).await?;
    let backup =
        backups::set_locked(&state, &session.context, id, backup_id, request.locked).await?;
    Ok(Json(backup_dto(backup)))
}

/// Restaure la sauvegarde (serveur arrêté d'abord) ; l'état suit (`restoring`, puis `offline`).
#[utoipa::path(post, path = "/api/client/v1/servers/{id}/backups/{backup_id}/restore", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("backup_id" = Uuid, Path)),
    request_body = RestoreBackupRequest,
    responses((status = 202), (status = 401, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 502, body = Problem)))]
pub async fn restore_backup(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path((id, backup_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<RestoreBackupRequest>,
) -> Result<StatusCode, ApiError> {
    owned(&state, &session, id).await?;
    let mode = match request.mode {
        RestoreModeDto::Overwrite => RestoreMode::Overwrite,
        RestoreModeDto::Clean => RestoreMode::Clean,
    };
    backups::restore(&state, &session.context, id, backup_id, mode).await?;
    Ok(StatusCode::ACCEPTED)
}

/// Supprime une sauvegarde non verrouillée.
#[utoipa::path(delete, path = "/api/client/v1/servers/{id}/backups/{backup_id}", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("backup_id" = Uuid, Path)),
    responses((status = 204), (status = 401, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 502, body = Problem)))]
pub async fn delete_backup(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path((id, backup_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    owned(&state, &session, id).await?;
    backups::delete(&state, &session.context, id, backup_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Ticket à usage unique (30 s) pour télécharger la sauvegarde par le formulaire
/// `…/backups/{backup_id}/download`.
#[utoipa::path(post, path = "/api/client/v1/servers/{id}/backups/{backup_id}/download/ticket", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("backup_id" = Uuid, Path)),
    responses((status = 201, body = DownloadTicketDto), (status = 401, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem)))]
pub async fn download_ticket(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path((id, backup_id)): Path<(Uuid, Uuid)>,
) -> Result<(StatusCode, Json<DownloadTicketDto>), ApiError> {
    owned(&state, &session, id).await?;
    let holder = TicketHolder::Owner(session.owner.as_str().to_owned());
    let ticket = backups::download_ticket(&state, &session.context, id, backup_id, &holder).await?;
    Ok((
        StatusCode::CREATED,
        Json(DownloadTicketDto {
            ticket,
            expires_in_secs: TICKET_TTL.as_secs(),
        }),
    ))
}

// Tâches planifiées.

/// Tâches planifiées du serveur.
#[utoipa::path(get, path = "/api/client/v1/servers/{id}/schedules", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [ScheduleDto]), (status = 401, body = Problem), (status = 404, body = Problem)))]
pub async fn list_schedules(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<ScheduleDto>>, ApiError> {
    owned(&state, &session, id).await?;
    let list = schedules::list(&state, id).await?;
    Ok(Json(list.into_iter().map(schedule_dto).collect()))
}

/// Crée une tâche ; une étape hors de l'espace client (action du prefab, mise à jour) est refusée.
#[utoipa::path(post, path = "/api/client/v1/servers/{id}/schedules", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = ScheduleRequest,
    responses((status = 201, body = ScheduleDto), (status = 401, body = Problem), (status = 403, body = Problem),
        (status = 404, body = Problem), (status = 409, body = Problem), (status = 422, body = Problem)))]
pub async fn create_schedule(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path(id): Path<Uuid>,
    Json(request): Json<ScheduleRequest>,
) -> Result<(StatusCode, Json<ScheduleDto>), ApiError> {
    owned(&state, &session, id).await?;
    let input = ScheduleInput::from(request);
    let view = schedules::create(&state, &session.context, &Authority::Owner, id, &input).await?;
    Ok((StatusCode::CREATED, Json(schedule_dto(view))))
}

/// Une tâche planifiée.
#[utoipa::path(get, path = "/api/client/v1/servers/{id}/schedules/{schedule_id}", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("schedule_id" = Uuid, Path)),
    responses((status = 200, body = ScheduleDto), (status = 401, body = Problem), (status = 404, body = Problem)))]
pub async fn get_schedule(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path((id, schedule_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<ScheduleDto>, ApiError> {
    owned(&state, &session, id).await?;
    Ok(Json(schedule_dto(
        schedules::get(&state, id, schedule_id).await?,
    )))
}

/// Remplace une tâche (activation comprise) ; mêmes règles qu'à la création.
#[utoipa::path(put, path = "/api/client/v1/servers/{id}/schedules/{schedule_id}", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("schedule_id" = Uuid, Path)),
    request_body = ScheduleRequest,
    responses((status = 200, body = ScheduleDto), (status = 401, body = Problem), (status = 403, body = Problem),
        (status = 404, body = Problem), (status = 422, body = Problem)))]
pub async fn update_schedule(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path((id, schedule_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<ScheduleRequest>,
) -> Result<Json<ScheduleDto>, ApiError> {
    owned(&state, &session, id).await?;
    let input = ScheduleInput::from(request);
    let view = schedules::update(
        &state,
        &session.context,
        &Authority::Owner,
        id,
        schedule_id,
        &input,
    )
    .await?;
    Ok(Json(schedule_dto(view)))
}

/// Supprime une tâche.
#[utoipa::path(delete, path = "/api/client/v1/servers/{id}/schedules/{schedule_id}", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("schedule_id" = Uuid, Path)),
    responses((status = 204), (status = 401, body = Problem), (status = 404, body = Problem)))]
pub async fn delete_schedule(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path((id, schedule_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    owned(&state, &session, id).await?;
    schedules::delete(&state, &session.context, id, schedule_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Lance la tâche tout de suite.
#[utoipa::path(post, path = "/api/client/v1/servers/{id}/schedules/{schedule_id}/run", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("schedule_id" = Uuid, Path)),
    responses((status = 202, body = RunDto), (status = 401, body = Problem), (status = 403, body = Problem),
        (status = 404, body = Problem), (status = 409, description = "La tâche tourne déjà", body = Problem)))]
pub async fn run_schedule(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path((id, schedule_id)): Path<(Uuid, Uuid)>,
) -> Result<(StatusCode, Json<RunDto>), ApiError> {
    owned(&state, &session, id).await?;
    let run =
        schedules::run_now(&state, &session.context, &Authority::Owner, id, schedule_id).await?;
    Ok((StatusCode::ACCEPTED, Json(run_dto(run))))
}

/// Dernières exécutions d'une tâche.
#[utoipa::path(get, path = "/api/client/v1/servers/{id}/schedules/{schedule_id}/runs", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("schedule_id" = Uuid, Path)),
    responses((status = 200, body = [RunDto]), (status = 401, body = Problem), (status = 404, body = Problem)))]
pub async fn schedule_runs(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path((id, schedule_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Vec<RunDto>>, ApiError> {
    owned(&state, &session, id).await?;
    let runs = schedules::runs(&state, id, schedule_id).await?;
    Ok(Json(runs.into_iter().map(run_dto).collect()))
}

/// Prochaines échéances d'une expression cron, dans un fuseau.
#[utoipa::path(post, path = "/api/client/v1/servers/{id}/schedules/preview", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = PreviewRequest,
    responses((status = 200, body = PreviewDto), (status = 401, body = Problem), (status = 404, body = Problem),
        (status = 422, body = Problem)))]
pub async fn preview_schedule(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path(id): Path<Uuid>,
    Json(request): Json<PreviewRequest>,
) -> Result<Json<PreviewDto>, ApiError> {
    owned(&state, &session, id).await?;
    let runs = schedules::preview(&request.cron, &request.timezone)?
        .into_iter()
        .filter_map(|run| {
            run.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .collect();
    Ok(Json(PreviewDto { runs }))
}
