//! Tâches planifiées d'un serveur (ADR-0022) : définition, lancement, historique, aperçu.

use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::audit::domain::{AuditAction, AuditEvent};
use crate::http::error::{ApiError, Problem};
use crate::http::extract::{Json, Path};
use crate::iam::application::IamState;
use crate::iam::domain::Permission;
use crate::iam::http::AdminSession;
use crate::servers::application::schedules::{
    self, ScheduleInput, ScheduleView, StepInput, StepKind,
};
use crate::servers::application::{ServersError, ServersState};
use crate::servers::domain::schedule::{StepAction, power_key};
use crate::servers::infra::RunRecord;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum StepActionDto {
    /// Ligne envoyée à la console (`servers.console.write`).
    Command,
    /// `start`, `stop`, `restart` ou `kill` (`servers.power`).
    Power,
    /// Sauvegarde, nommée ou datée ; l'étape attend sa fin (`servers.backups.create`).
    Backup,
    /// Action planifiable du prefab, par sa clé (`servers.actions.run`, ADR-0039).
    Action,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct StepDto {
    pub action: StepActionDto,
    /// Ligne de commande, action d'alimentation, ou nom de la sauvegarde (facultatif).
    pub value: Option<String>,
    /// Attente avant l'étape, de 0 à 3600 s.
    #[serde(default)]
    pub delay_secs: i32,
    /// Poursuivre la tâche si cette étape échoue.
    #[serde(default)]
    pub continue_on_failure: bool,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunTriggerDto {
    Schedule,
    Manual,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcomeDto {
    Running,
    Succeeded,
    Failed,
    Skipped,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RunDto {
    pub id: Uuid,
    pub trigger: RunTriggerDto,
    pub outcome: RunOutcomeDto,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub started_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub finished_at: Option<OffsetDateTime>,
    /// Position de la première étape en échec.
    pub failed_step: Option<i32>,
    /// Code de la cause : un code d'erreur de l'API, ou `late`, `overlap`, `suspended`,
    /// `offline`, `backup_failed`, `timeout`, `interrupted`.
    pub reason: Option<String>,
}

fn run_dto(run: RunRecord) -> RunDto {
    RunDto {
        id: run.id,
        trigger: if run.trigger == "manual" {
            RunTriggerDto::Manual
        } else {
            RunTriggerDto::Schedule
        },
        outcome: match run.outcome.as_str() {
            "running" => RunOutcomeDto::Running,
            "succeeded" => RunOutcomeDto::Succeeded,
            "skipped" => RunOutcomeDto::Skipped,
            _ => RunOutcomeDto::Failed,
        },
        started_at: run.started_at,
        finished_at: run.finished_at,
        failed_step: run.failed_step,
        reason: run.reason,
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ScheduleDto {
    pub id: Uuid,
    pub name: String,
    /// Expression à 5 champs.
    #[schema(example = "0 4 * * *")]
    pub cron: String,
    /// Fuseau IANA dans lequel l'expression est lue.
    #[schema(example = "Europe/Paris")]
    pub timezone: String,
    pub enabled: bool,
    pub only_when_online: bool,
    /// Absente quand la tâche est désactivée.
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub next_run_at: Option<OffsetDateTime>,
    /// Une exécution est en cours.
    pub running: bool,
    pub steps: Vec<StepDto>,
    pub last_run: Option<RunDto>,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
}

fn step_dto(step: &crate::servers::domain::schedule::Step) -> StepDto {
    let (action, value) = match &step.action {
        StepAction::Command(line) => (StepActionDto::Command, Some(line.clone())),
        StepAction::Power(action) => (StepActionDto::Power, Some(power_key(*action).to_owned())),
        StepAction::Backup(name) => (StepActionDto::Backup, name.clone()),
        StepAction::Action(key) => (StepActionDto::Action, Some(key.clone())),
    };
    StepDto {
        action,
        value,
        delay_secs: step.delay_secs,
        continue_on_failure: step.continue_on_failure,
    }
}

fn dto(view: ScheduleView) -> ScheduleDto {
    ScheduleDto {
        id: view.schedule.id,
        name: view.schedule.name,
        cron: view.schedule.cron,
        timezone: view.schedule.timezone,
        enabled: view.schedule.enabled,
        only_when_online: view.schedule.only_when_online,
        next_run_at: view.schedule.next_run_at,
        running: view.schedule.running_since.is_some(),
        steps: view.steps.iter().map(step_dto).collect(),
        last_run: view.last_run.map(run_dto),
        created_at: view.schedule.created_at,
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ScheduleRequest {
    #[schema(example = "Sauvegarde nocturne")]
    pub name: String,
    #[schema(example = "0 4 * * *")]
    pub cron: String,
    #[schema(example = "Europe/Paris")]
    pub timezone: String,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    #[serde(default)]
    pub only_when_online: bool,
    /// De 1 à 10 étapes, exécutées dans l'ordre.
    pub steps: Vec<StepDto>,
}

fn enabled_by_default() -> bool {
    true
}

impl From<ScheduleRequest> for ScheduleInput {
    fn from(request: ScheduleRequest) -> Self {
        Self {
            name: request.name,
            cron: request.cron,
            timezone: request.timezone,
            enabled: request.enabled,
            only_when_online: request.only_when_online,
            steps: request
                .steps
                .into_iter()
                .map(|step| StepInput {
                    kind: match step.action {
                        StepActionDto::Command => StepKind::Command,
                        StepActionDto::Power => StepKind::Power,
                        StepActionDto::Backup => StepKind::Backup,
                        StepActionDto::Action => StepKind::Action,
                    },
                    value: step.value,
                    delay_secs: step.delay_secs,
                    continue_on_failure: step.continue_on_failure,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PreviewRequest {
    #[schema(example = "0 */6 * * *")]
    pub cron: String,
    #[schema(example = "Europe/Paris")]
    pub timezone: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PreviewDto {
    /// Les 3 prochaines échéances (UTC).
    #[schema(value_type = Vec<String>)]
    pub runs: Vec<String>,
}

/// Une étape que l'admin n'a pas le droit de faire : refus journalisé comme les autres.
async fn audited<T>(
    iam: &IamState,
    session: &AdminSession,
    result: Result<T, ServersError>,
) -> Result<T, ApiError> {
    if let Err(ServersError::Forbidden(permission)) = &result {
        let event = AuditEvent::denied(AuditAction::AccessDenied)
            .details(json!({ "permission": permission.key() }));
        if let Err(error) =
            crate::audit::application::record(&iam.pool, &session.context, event).await
        {
            tracing::error!(%error, "failed to record access denial");
        }
    }
    Ok(result?)
}

/// Tâches planifiées du serveur et leur dernière exécution. Permission : `servers.schedules.read`.
#[utoipa::path(get, path = "/api/admin/v1/servers/{id}/schedules", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [ScheduleDto]), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn list(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<ScheduleDto>>, ApiError> {
    session
        .require(&iam, Permission::ServersSchedulesRead)
        .await?;
    let list = schedules::list(&state, id).await?;
    Ok(Json(list.into_iter().map(dto).collect()))
}

/// Crée une tâche (20 par serveur au plus). Ses étapes exigent aussi leurs propres permissions.
/// Permission : `servers.schedules.manage`.
#[utoipa::path(post, path = "/api/admin/v1/servers/{id}/schedules", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = ScheduleRequest,
    responses((status = 201, body = ScheduleDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 422, body = Problem)))]
pub async fn create(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Json(request): Json<ScheduleRequest>,
) -> Result<(StatusCode, Json<ScheduleDto>), ApiError> {
    let authority = session
        .require(&iam, Permission::ServersSchedulesManage)
        .await?;
    let input = ScheduleInput::from(request);
    let result = schedules::create(&state, &session.context, &authority, id, &input).await;
    let view = audited(&iam, &session, result).await?;
    Ok((StatusCode::CREATED, Json(dto(view))))
}

/// Une tâche. Permission : `servers.schedules.read`.
#[utoipa::path(get, path = "/api/admin/v1/servers/{id}/schedules/{schedule_id}", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("schedule_id" = Uuid, Path)),
    responses((status = 200, body = ScheduleDto), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn get(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path((id, schedule_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<ScheduleDto>, ApiError> {
    session
        .require(&iam, Permission::ServersSchedulesRead)
        .await?;
    Ok(Json(dto(schedules::get(&state, id, schedule_id).await?)))
}

/// Remplace la définition ; l'auteur doit détenir les permissions de toutes les étapes.
/// Permission : `servers.schedules.manage`.
#[utoipa::path(put, path = "/api/admin/v1/servers/{id}/schedules/{schedule_id}", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("schedule_id" = Uuid, Path)),
    request_body = ScheduleRequest,
    responses((status = 200, body = ScheduleDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 422, body = Problem)))]
pub async fn update(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path((id, schedule_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<ScheduleRequest>,
) -> Result<Json<ScheduleDto>, ApiError> {
    let authority = session
        .require(&iam, Permission::ServersSchedulesManage)
        .await?;
    let input = ScheduleInput::from(request);
    let result = schedules::update(
        &state,
        &session.context,
        &authority,
        id,
        schedule_id,
        &input,
    )
    .await;
    Ok(Json(dto(audited(&iam, &session, result).await?)))
}

/// Supprime une tâche et son historique. Permission : `servers.schedules.manage`.
#[utoipa::path(delete, path = "/api/admin/v1/servers/{id}/schedules/{schedule_id}", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("schedule_id" = Uuid, Path)),
    responses((status = 204), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn delete(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path((id, schedule_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    session
        .require(&iam, Permission::ServersSchedulesManage)
        .await?;
    schedules::delete(&state, &session.context, id, schedule_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Lance la tâche tout de suite. Permission : `servers.schedules.manage`, et celles des étapes.
#[utoipa::path(post, path = "/api/admin/v1/servers/{id}/schedules/{schedule_id}/run", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("schedule_id" = Uuid, Path)),
    responses((status = 202, body = RunDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, description = "La tâche tourne déjà", body = Problem)))]
pub async fn run(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path((id, schedule_id)): Path<(Uuid, Uuid)>,
) -> Result<(StatusCode, Json<RunDto>), ApiError> {
    let authority = session
        .require(&iam, Permission::ServersSchedulesManage)
        .await?;
    let result = schedules::run_now(&state, &session.context, &authority, id, schedule_id).await;
    let run = audited(&iam, &session, result).await?;
    Ok((StatusCode::ACCEPTED, Json(run_dto(run))))
}

/// Les 20 dernières exécutions, de la plus récente à la plus ancienne.
/// Permission : `servers.schedules.read`.
#[utoipa::path(get, path = "/api/admin/v1/servers/{id}/schedules/{schedule_id}/runs", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("schedule_id" = Uuid, Path)),
    responses((status = 200, body = [RunDto]), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn runs(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path((id, schedule_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Vec<RunDto>>, ApiError> {
    session
        .require(&iam, Permission::ServersSchedulesRead)
        .await?;
    let runs = schedules::runs(&state, id, schedule_id).await?;
    Ok(Json(runs.into_iter().map(run_dto).collect()))
}

/// Prochaines échéances d'une expression dans un fuseau, pour l'éditeur.
/// Permission : `servers.schedules.read`.
#[utoipa::path(post, path = "/api/admin/v1/servers/{id}/schedules/preview", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = PreviewRequest,
    responses((status = 200, body = PreviewDto), (status = 403, body = Problem), (status = 422, body = Problem)))]
pub async fn preview(
    State(iam): State<IamState>,
    session: AdminSession,
    Path(_id): Path<Uuid>,
    Json(request): Json<PreviewRequest>,
) -> Result<Json<PreviewDto>, ApiError> {
    session
        .require(&iam, Permission::ServersSchedulesRead)
        .await?;
    let runs = schedules::preview(&request.cron, &request.timezone)?;
    let runs = runs
        .into_iter()
        .filter_map(|run| {
            run.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .collect();
    Ok(Json(PreviewDto { runs }))
}
