//! Tâches planifiées d'un serveur (ADR-0022) : définition, anti-escalade, lancement manuel,
//! historique et aperçu des échéances. L'exécution elle-même est dans `scheduler`.

use serde_json::json;
use time::OffsetDateTime;
use uuid::Uuid;

use super::{ServersError, ServersState, scheduler};
use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::error_code::{ErrorCode, FieldError};
use crate::iam::domain::{Authority, Permission};
use crate::servers::domain::schedule::{
    MAX_DELAY_SECS, MAX_NAME_CHARS, MAX_SCHEDULES_PER_SERVER, MAX_STEPS, RUNS_KEPT, Step,
    StepAction, Trigger, TriggerError, power_from_key,
};
use crate::servers::infra::{
    RunRecord, ScheduleFields, ScheduleRecord, ScheduleRepository, ServerRepository, StepRecord,
};

/// Échéances montrées par l'aperçu.
const PREVIEW_RUNS: usize = 3;

/// Tâche, ses étapes et sa dernière exécution.
#[derive(Debug, Clone)]
pub struct ScheduleView {
    pub schedule: ScheduleRecord,
    pub steps: Vec<Step>,
    pub last_run: Option<RunRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    Command,
    Power,
    Backup,
    Action,
}

/// Étape demandée, validée par [`create`] ou [`update`].
#[derive(Debug, Clone)]
pub struct StepInput {
    pub kind: StepKind,
    /// Ligne de commande, action d'alimentation ou nom de la sauvegarde.
    pub value: Option<String>,
    pub delay_secs: i32,
    pub continue_on_failure: bool,
}

#[derive(Debug, Clone)]
pub struct ScheduleInput {
    pub name: String,
    pub cron: String,
    pub timezone: String,
    pub enabled: bool,
    pub only_when_online: bool,
    pub steps: Vec<StepInput>,
}

/// Permission qu'exige une étape : une tâche ne fait jamais ce que son auteur ne peut pas faire.
pub fn step_permission(action: &StepAction) -> Permission {
    match action {
        StepAction::Command(_) => Permission::ServersConsoleWrite,
        StepAction::Power(_) => Permission::ServersPower,
        StepAction::Backup(_) => Permission::ServersBackupsCreate,
        StepAction::Action(_) => Permission::ServersActionsRun,
    }
}

fn require_steps(authority: &Authority, steps: &[Step]) -> Result<(), ServersError> {
    for step in steps {
        let permission = step_permission(&step.action);
        if !authority.has(permission) {
            return Err(ServersError::Forbidden(permission));
        }
    }
    Ok(())
}

fn invalid(field: impl Into<std::borrow::Cow<'static, str>>, code: ErrorCode) -> ServersError {
    ServersError::Invalid(FieldError::new(field, code))
}

pub fn parse_trigger(cron: &str, timezone: &str) -> Result<Trigger, ServersError> {
    Trigger::parse(cron.trim(), timezone.trim()).map_err(|error| match error {
        TriggerError::Cron => invalid("cron", ErrorCode::InvalidCron),
        TriggerError::Timezone => invalid("timezone", ErrorCode::InvalidTimezone),
    })
}

fn validate_steps(inputs: &[StepInput]) -> Result<Vec<Step>, ServersError> {
    if inputs.is_empty() || inputs.len() > MAX_STEPS {
        return Err(ServersError::Invalid(
            FieldError::new("steps", ErrorCode::InvalidScheduleStep).param("max", MAX_STEPS),
        ));
    }
    inputs
        .iter()
        .enumerate()
        .map(|(index, input)| {
            let field = |name: &str| format!("steps.{index}.{name}");
            if !(0..=MAX_DELAY_SECS).contains(&input.delay_secs) {
                return Err(ServersError::Invalid(
                    FieldError::new(field("delay_secs"), ErrorCode::InvalidScheduleStep)
                        .param("max", MAX_DELAY_SECS),
                ));
            }
            let value = input
                .value
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty());
            let action = match (input.kind, value) {
                (StepKind::Command, Some(line))
                    if kuro_protocol::limits::is_valid_command(line) =>
                {
                    StepAction::Command(line.to_owned())
                }
                (StepKind::Power, Some(key)) => match power_from_key(key) {
                    Some(action) => StepAction::Power(action),
                    None => return Err(invalid(field("value"), ErrorCode::InvalidScheduleStep)),
                },
                (StepKind::Backup, None) => StepAction::Backup(None),
                (StepKind::Backup, Some(name))
                    if crate::validation::is_display_text(name, 1, MAX_NAME_CHARS) =>
                {
                    StepAction::Backup(Some(name.to_owned()))
                }
                (StepKind::Action, Some(key)) if kuro_prefab::template::is_slug(key) => {
                    StepAction::Action(key.to_owned())
                }
                _ => return Err(invalid(field("value"), ErrorCode::InvalidScheduleStep)),
            };
            Ok(Step {
                action,
                delay_secs: input.delay_secs,
                continue_on_failure: input.continue_on_failure,
            })
        })
        .collect()
}

/// Définition validée : nom, déclencheur et étapes.
struct Validated {
    name: String,
    cron: String,
    timezone: String,
    trigger: Trigger,
    steps: Vec<Step>,
}

fn validate(input: &ScheduleInput) -> Result<Validated, ServersError> {
    let name = input.name.trim();
    if !crate::validation::is_display_text(name, 1, MAX_NAME_CHARS) {
        return Err(invalid("name", ErrorCode::InvalidScheduleName));
    }
    let trigger = parse_trigger(&input.cron, &input.timezone)?;
    if trigger.next_after(OffsetDateTime::now_utc()).is_none() {
        return Err(invalid("cron", ErrorCode::InvalidCron));
    }
    Ok(Validated {
        name: name.to_owned(),
        cron: input.cron.trim().to_owned(),
        timezone: input.timezone.trim().to_owned(),
        trigger,
        steps: validate_steps(&input.steps)?,
    })
}

/// Étapes stockées, relues ; une ligne incohérente est ignorée (et journalisée).
pub(super) fn steps_of(records: &[StepRecord], schedule_id: Uuid) -> Vec<Step> {
    records
        .iter()
        .filter(|step| step.schedule_id == schedule_id)
        .filter_map(|step| {
            let action = StepAction::from_stored(&step.action, step.payload.as_deref());
            if action.is_none() {
                tracing::error!(%schedule_id, position = step.position, "unreadable schedule step");
            }
            Some(Step {
                action: action?,
                delay_secs: step.delay_secs,
                continue_on_failure: step.continue_on_failure,
            })
        })
        .collect()
}

async fn ensure_server(conn: &mut sqlx::PgConnection, server_id: Uuid) -> Result<(), ServersError> {
    ServerRepository::get(conn, server_id)
        .await?
        .map(|_| ())
        .ok_or(ServersError::NotFound)
}

pub async fn list(
    state: &ServersState,
    server_id: Uuid,
) -> Result<Vec<ScheduleView>, ServersError> {
    let mut conn = state.pool.acquire().await?;
    ensure_server(&mut conn, server_id).await?;
    let schedules = ScheduleRepository::of_server(&mut conn, server_id).await?;
    let ids: Vec<Uuid> = schedules.iter().map(|s| s.id).collect();
    let steps = ScheduleRepository::steps(&mut conn, &ids).await?;
    let mut last_runs = ScheduleRepository::last_runs(&mut conn, server_id).await?;
    Ok(schedules
        .into_iter()
        .map(|schedule| {
            let last_run = last_runs
                .iter()
                .position(|run| run.schedule_id == schedule.id)
                .map(|index| last_runs.swap_remove(index));
            ScheduleView {
                steps: steps_of(&steps, schedule.id),
                schedule,
                last_run,
            }
        })
        .collect())
}

pub async fn get(
    state: &ServersState,
    server_id: Uuid,
    id: Uuid,
) -> Result<ScheduleView, ServersError> {
    let mut conn = state.pool.acquire().await?;
    let schedule = ScheduleRepository::get(&mut conn, server_id, id)
        .await?
        .ok_or(ServersError::Missing(ErrorCode::ScheduleNotFound))?;
    let steps = ScheduleRepository::steps(&mut conn, &[id]).await?;
    let last_run = ScheduleRepository::runs(&mut conn, id, 1).await?.pop();
    Ok(ScheduleView {
        steps: steps_of(&steps, id),
        schedule,
        last_run,
    })
}

fn audit_details(validated: &Validated, enabled: bool) -> serde_json::Value {
    json!({
        "name": validated.name,
        "cron": validated.cron,
        "timezone": validated.timezone,
        "enabled": enabled,
        // Le type des étapes seulement : une commande peut contenir un secret.
        "steps": validated.steps.iter().map(|step| step.action.kind()).collect::<Vec<_>>(),
    })
}

/// Une étape « action » vise une action planifiable du prefab du serveur (ADR-0039).
async fn check_action_steps(
    state: &ServersState,
    server_id: Uuid,
    steps: &[Step],
) -> Result<(), ServersError> {
    let keys: Vec<(usize, &str)> = steps
        .iter()
        .enumerate()
        .filter_map(|(index, step)| match &step.action {
            StepAction::Action(key) => Some((index, key.as_str())),
            _ => None,
        })
        .collect();
    if keys.is_empty() {
        return Ok(());
    }
    let server = super::get(state, server_id).await?.server;
    let package = crate::prefabs::application::package(
        &state.pool,
        &server.prefab_id,
        &server.prefab_version,
    )
    .await?;
    for (index, key) in keys {
        if !super::actions::is_schedulable(&package.manifest, key) {
            return Err(invalid(
                format!("steps.{index}.value"),
                ErrorCode::ActionNotSchedulable,
            ));
        }
    }
    Ok(())
}

pub async fn create(
    state: &ServersState,
    context: &AuditContext,
    authority: &Authority,
    server_id: Uuid,
    input: &ScheduleInput,
) -> Result<ScheduleView, ServersError> {
    let validated = validate(input)?;
    require_steps(authority, &validated.steps)?;
    check_action_steps(state, server_id, &validated.steps).await?;
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    if !ServerRepository::lock(&mut tx, server_id).await? {
        return Err(ServersError::NotFound);
    }
    if ScheduleRepository::count_for_server(&mut tx, server_id).await? >= MAX_SCHEDULES_PER_SERVER {
        return Err(ServersError::Conflict(ErrorCode::ScheduleLimitReached));
    }
    let id = Uuid::now_v7();
    let next_run_at = input
        .enabled
        .then(|| validated.trigger.next_after(OffsetDateTime::now_utc()))
        .flatten();
    let fields = ScheduleFields {
        name: &validated.name,
        cron: &validated.cron,
        timezone: &validated.timezone,
        enabled: input.enabled,
        only_when_online: input.only_when_online,
        next_run_at,
    };
    ScheduleRepository::insert(&mut tx, id, server_id, context.actor.id, &fields).await?;
    ScheduleRepository::replace_steps(&mut tx, id, &validated.steps).await?;
    tx.commit(
        AuditEvent::success(AuditAction::ServerScheduleCreated)
            .target("server", server_id)
            .details(
                json!({ "schedule_id": id, "schedule": audit_details(&validated, input.enabled) }),
            ),
    )
    .await?;
    get(state, server_id, id).await
}

/// Remplace la définition. L'auteur doit détenir les permissions de toutes les étapes qui
/// resteront, même celles qu'il ne change pas.
pub async fn update(
    state: &ServersState,
    context: &AuditContext,
    authority: &Authority,
    server_id: Uuid,
    id: Uuid,
    input: &ScheduleInput,
) -> Result<ScheduleView, ServersError> {
    let validated = validate(input)?;
    require_steps(authority, &validated.steps)?;
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    ScheduleRepository::lock(&mut tx, server_id, id)
        .await?
        .ok_or(ServersError::Missing(ErrorCode::ScheduleNotFound))?;
    let next_run_at = input
        .enabled
        .then(|| validated.trigger.next_after(OffsetDateTime::now_utc()))
        .flatten();
    let fields = ScheduleFields {
        name: &validated.name,
        cron: &validated.cron,
        timezone: &validated.timezone,
        enabled: input.enabled,
        only_when_online: input.only_when_online,
        next_run_at,
    };
    ScheduleRepository::update(&mut tx, id, &fields).await?;
    ScheduleRepository::replace_steps(&mut tx, id, &validated.steps).await?;
    tx.commit(
        AuditEvent::success(AuditAction::ServerScheduleUpdated)
            .target("server", server_id)
            .details(
                json!({ "schedule_id": id, "schedule": audit_details(&validated, input.enabled) }),
            ),
    )
    .await?;
    get(state, server_id, id).await
}

pub async fn delete(
    state: &ServersState,
    context: &AuditContext,
    server_id: Uuid,
    id: Uuid,
) -> Result<(), ServersError> {
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let schedule = ScheduleRepository::lock(&mut tx, server_id, id)
        .await?
        .ok_or(ServersError::Missing(ErrorCode::ScheduleNotFound))?;
    ScheduleRepository::delete(&mut tx, id).await?;
    tx.commit(
        AuditEvent::success(AuditAction::ServerScheduleDeleted)
            .target("server", server_id)
            .details(json!({ "schedule_id": id, "name": schedule.name })),
    )
    .await?;
    Ok(())
}

/// Lance la tâche tout de suite, hors échéance. Refusé si elle tourne déjà ; ses étapes
/// exigent les mêmes permissions qu'à la création.
pub async fn run_now(
    state: &ServersState,
    context: &AuditContext,
    authority: &Authority,
    server_id: Uuid,
    id: Uuid,
) -> Result<RunRecord, ServersError> {
    let view = get(state, server_id, id).await?;
    require_steps(authority, &view.steps)?;
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let schedule = ScheduleRepository::lock(&mut tx, server_id, id)
        .await?
        .ok_or(ServersError::Missing(ErrorCode::ScheduleNotFound))?;
    if schedule.running_since.is_some() {
        return Err(ServersError::Conflict(ErrorCode::ScheduleRunning));
    }
    let now = OffsetDateTime::now_utc();
    let run = scheduler::open_run(&mut tx, id, "manual", now).await?;
    tx.commit(
        AuditEvent::success(AuditAction::ServerScheduleStarted)
            .target("server", server_id)
            .details(json!({ "schedule_id": id, "run_id": run.id })),
    )
    .await?;
    scheduler::spawn(state.clone(), schedule, view.steps, run.id);
    Ok(run)
}

pub async fn runs(
    state: &ServersState,
    server_id: Uuid,
    id: Uuid,
) -> Result<Vec<RunRecord>, ServersError> {
    let mut conn = state.pool.acquire().await?;
    ScheduleRepository::get(&mut conn, server_id, id)
        .await?
        .ok_or(ServersError::Missing(ErrorCode::ScheduleNotFound))?;
    Ok(ScheduleRepository::runs(&mut conn, id, RUNS_KEPT).await?)
}

/// Prochaines échéances d'une expression, pour l'éditeur du panel.
pub fn preview(cron: &str, timezone: &str) -> Result<Vec<OffsetDateTime>, ServersError> {
    let runs = parse_trigger(cron, timezone)?.upcoming(OffsetDateTime::now_utc(), PREVIEW_RUNS);
    if runs.is_empty() {
        return Err(invalid("cron", ErrorCode::InvalidCron));
    }
    Ok(runs)
}
