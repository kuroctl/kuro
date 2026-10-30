//! Exécution des tâches planifiées (ADR-0022), par l'API elle-même.
//!
//! Une boucle dort jusqu'à la prochaine échéance (30 s au plus) puis prend les tâches dues avec
//! `FOR UPDATE SKIP LOCKED` : plusieurs instances de l'API ne lancent jamais deux fois la même.
//! Une échéance manquée de plus de 5 min (API arrêtée) est sautée, sans rattrapage en rafale. Une
//! tâche ne se chevauche jamais elle-même. Les étapes passent par les cas d'usage existants
//! (commande, alimentation, sauvegarde), avec l'acteur d'audit « système, tâche X ».

use std::time::Duration;

use serde_json::json;
use sqlx::PgConnection;
use time::OffsetDateTime;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::schedules::{parse_trigger, steps_of};
use super::{ServersError, ServersState, backups};
use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::servers::domain::ServerState;
use crate::servers::domain::schedule::{RUNS_KEPT, Step, StepAction};
use crate::servers::infra::{
    BackupRepository, RunRecord, ScheduleRecord, ScheduleRepository, ServerRepository,
};

/// Sommeil maximal de la boucle : une tâche créée ou modifiée est vue dans ce délai.
const MAX_SLEEP: Duration = Duration::from_secs(30);
/// Nouvel essai après une base injoignable : sans attendre une échéance qu'on ne connaît pas.
const RETRY_SLEEP: Duration = Duration::from_secs(5);
/// Retard au-delà duquel une échéance manquée est sautée.
const LATE_TOLERANCE: time::Duration = time::Duration::minutes(5);
/// Tâches prises par passage.
const BATCH: i64 = 50;
/// Attente de la fin d'une sauvegarde.
const BACKUP_WAIT: Duration = Duration::from_secs(60 * 60);
/// Lecture de l'état de la sauvegarde en cours : une requête par tour, le nœud met des minutes.
const BACKUP_POLL: Duration = Duration::from_secs(3);

/// Ouvre une exécution `running` et marque la tâche occupée.
pub(super) async fn open_run(
    conn: &mut PgConnection,
    schedule_id: Uuid,
    trigger: &str,
    now: OffsetDateTime,
) -> Result<RunRecord, ServersError> {
    let run = RunRecord {
        id: Uuid::now_v7(),
        schedule_id,
        trigger: trigger.to_owned(),
        started_at: now,
        finished_at: None,
        outcome: "running".to_owned(),
        failed_step: None,
        reason: None,
    };
    ScheduleRepository::insert_run(&mut *conn, &run).await?;
    ScheduleRepository::set_running(conn, schedule_id, Some(now)).await?;
    Ok(run)
}

/// Échéance sautée : notée dans l'historique, sans rien lancer.
async fn skip(
    conn: &mut PgConnection,
    schedule_id: Uuid,
    now: OffsetDateTime,
    reason: &str,
) -> Result<(), ServersError> {
    ScheduleRepository::insert_run(
        &mut *conn,
        &RunRecord {
            id: Uuid::now_v7(),
            schedule_id,
            trigger: "schedule".to_owned(),
            started_at: now,
            finished_at: Some(now),
            outcome: "skipped".to_owned(),
            failed_step: None,
            reason: Some(reason.to_owned()),
        },
    )
    .await?;
    ScheduleRepository::prune_runs(conn, schedule_id, RUNS_KEPT).await?;
    Ok(())
}

/// Prend les tâches dues à `now` et lance leurs exécutions. Les tâches lancées sont rendues pour
/// que les tests puissent les attendre.
pub async fn run_due(
    state: &ServersState,
    now: OffsetDateTime,
) -> Result<Vec<JoinHandle<()>>, ServersError> {
    let mut tx = state.pool.begin().await?;
    let due = ScheduleRepository::lock_due(&mut tx, now, BATCH).await?;
    let ids: Vec<Uuid> = due.iter().map(|schedule| schedule.id).collect();
    let steps = ScheduleRepository::steps(&mut tx, &ids).await?;
    let mut started = Vec::new();
    for schedule in due {
        let next = parse_trigger(&schedule.cron, &schedule.timezone)
            .ok()
            .and_then(|trigger| trigger.next_after(now));
        ScheduleRepository::set_next_run(&mut tx, schedule.id, next).await?;
        let late = schedule
            .next_run_at
            .is_some_and(|due_at| now - due_at > LATE_TOLERANCE);
        if late {
            skip(&mut tx, schedule.id, now, "late").await?;
        } else if schedule.running_since.is_some() {
            skip(&mut tx, schedule.id, now, "overlap").await?;
        } else {
            let run = open_run(&mut tx, schedule.id, "schedule", now).await?;
            let schedule_steps = steps_of(&steps, schedule.id);
            started.push((schedule, schedule_steps, run.id));
        }
    }
    tx.commit().await?;
    Ok(started
        .into_iter()
        .map(|(schedule, steps, run_id)| spawn(state.clone(), schedule, steps, run_id))
        .collect())
}

/// Lance l'exécution en tâche de fond.
pub(super) fn spawn(
    state: ServersState,
    schedule: ScheduleRecord,
    steps: Vec<Step>,
    run_id: Uuid,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let schedule_id = schedule.id;
        if let Err(error) = execute(&state, schedule, &steps, run_id).await {
            tracing::error!(%schedule_id, %run_id, %error, "scheduled task failed to record");
        }
    })
}

/// Résultat d'une exécution : réussie, sautée ou en échec (étape et raison).
enum Outcome {
    Succeeded,
    Skipped(&'static str),
    Failed { step: Option<i32>, reason: String },
}

async fn execute(
    state: &ServersState,
    schedule: ScheduleRecord,
    steps: &[Step],
    run_id: Uuid,
) -> Result<(), ServersError> {
    let context = AuditContext::scheduler(schedule.id);
    let outcome = run_steps(state, &context, &schedule, steps).await;
    let (label, failed_step, reason) = match &outcome {
        Outcome::Succeeded => ("succeeded", None, None),
        Outcome::Skipped(reason) => ("skipped", None, Some((*reason).to_owned())),
        Outcome::Failed { step, reason } => ("failed", *step, Some(reason.clone())),
    };
    let mut tx = AuditedTx::begin(&state.pool, &context).await?;
    ScheduleRepository::finish_run(&mut tx, run_id, label, failed_step, reason.as_deref()).await?;
    ScheduleRepository::set_running(&mut tx, schedule.id, None).await?;
    ScheduleRepository::prune_runs(&mut tx, schedule.id, RUNS_KEPT).await?;
    let details = json!({
        "schedule_id": schedule.id,
        "run_id": run_id,
        "outcome": label,
        "failed_step": failed_step,
        "reason": reason,
    });
    let event = match outcome {
        Outcome::Failed { .. } => AuditEvent::failure(AuditAction::ServerScheduleRun),
        _ => AuditEvent::success(AuditAction::ServerScheduleRun),
    };
    tx.commit(event.target("server", schedule.server_id).details(details))
        .await?;
    Ok(())
}

async fn run_steps(
    state: &ServersState,
    context: &AuditContext,
    schedule: &ScheduleRecord,
    steps: &[Step],
) -> Outcome {
    let server = {
        let mut conn = match state.pool.acquire().await {
            Ok(conn) => conn,
            Err(error) => return failed(None, &ServersError::from(error)),
        };
        match ServerRepository::get(&mut conn, schedule.server_id).await {
            Ok(Some(server)) => server,
            Ok(None) => return failed(None, &ServersError::NotFound),
            Err(error) => return failed(None, &ServersError::from(error)),
        }
    };
    if server.suspended_at.is_some() {
        return Outcome::Skipped("suspended");
    }
    if schedule.only_when_online && server.state != ServerState::Running.as_str() {
        return Outcome::Skipped("offline");
    }
    let mut first_failure = None;
    for (position, step) in steps.iter().enumerate() {
        if step.delay_secs > 0 {
            tokio::time::sleep(Duration::from_secs(step.delay_secs.unsigned_abs().into())).await;
        }
        if let Err(reason) = run_step(state, context, schedule.server_id, &step.action).await {
            let position = i32::try_from(position).unwrap_or(i32::MAX);
            first_failure.get_or_insert((position, reason));
            if !step.continue_on_failure {
                break;
            }
        }
    }
    match first_failure {
        None => Outcome::Succeeded,
        Some((step, reason)) => Outcome::Failed {
            step: Some(step),
            reason,
        },
    }
}

fn failed(step: Option<i32>, error: &ServersError) -> Outcome {
    Outcome::Failed {
        step,
        reason: reason_code(error),
    }
}

/// Code stable de la cause d'un échec, traduit par le panel.
pub(super) fn reason_code(error: &ServersError) -> String {
    match error {
        ServersError::NotFound => "not_found".to_owned(),
        ServersError::Missing(code)
        | ServersError::Unprocessable(code)
        | ServersError::Conflict(code)
        | ServersError::Daemon { code, .. } => code.as_str().to_owned(),
        ServersError::Invalid(field) => field.code.as_str().to_owned(),
        ServersError::InvalidSettings(_) => "invalid_settings".to_owned(),
        ServersError::Forbidden(_) => "forbidden".to_owned(),
        ServersError::Internal(_) => "internal".to_owned(),
    }
}

/// Une étape ; `Err` porte le code de la cause.
async fn run_step(
    state: &ServersState,
    context: &AuditContext,
    server_id: Uuid,
    action: &StepAction,
) -> Result<(), String> {
    match action {
        StepAction::Command(line) => super::command(state, context, server_id, line)
            .await
            .map_err(|error| reason_code(&error)),
        StepAction::Power(power) => super::power(state, context, server_id, *power)
            .await
            .map_err(|error| reason_code(&error)),
        StepAction::Backup(name) => {
            let backup = backups::create(state, context, server_id, name.as_deref())
                .await
                .map_err(|error| reason_code(&error))?;
            wait_for_backup(state, server_id, backup.id).await
        }
    }
}

/// La sauvegarde est rapportée par le nœud : l'étape suivante attend qu'elle soit prête.
pub(super) async fn wait_for_backup(
    state: &ServersState,
    server_id: Uuid,
    id: Uuid,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + BACKUP_WAIT;
    loop {
        let backup = {
            let mut conn = state
                .pool
                .acquire()
                .await
                .map_err(|_| "internal".to_owned())?;
            BackupRepository::get(&mut conn, server_id, id)
                .await
                .map_err(|_| "internal".to_owned())?
        };
        match backup.as_ref().map(|backup| backup.state.as_str()) {
            Some("ready") => return Ok(()),
            Some("creating") => {}
            // Échouée, ou supprimée entre-temps.
            _ => return Err("backup_failed".to_owned()),
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("timeout".to_owned());
        }
        tokio::time::sleep(BACKUP_POLL).await;
    }
}

/// Boucle de l'API : reprise après un arrêt, puis exécutions à chaque échéance.
pub async fn run_forever(state: ServersState, shutdown: CancellationToken) {
    match state.pool.acquire().await {
        Ok(mut conn) => match ScheduleRepository::recover(&mut conn).await {
            Ok(0) => {}
            Ok(count) => tracing::warn!(count, "scheduled runs interrupted by a restart"),
            Err(error) => tracing::error!(%error, "failed to recover scheduled runs"),
        },
        Err(error) => tracing::error!(%error, "failed to recover scheduled runs"),
    }
    loop {
        let now = OffsetDateTime::now_utc();
        if let Err(error) = run_due(&state, now).await {
            tracing::error!(%error, "scheduled tasks pass failed");
        }
        let sleep = match next_due(&state).await {
            Ok(Some(next)) => (next - OffsetDateTime::now_utc())
                .try_into()
                .unwrap_or(Duration::ZERO)
                .min(MAX_SLEEP),
            Ok(None) => MAX_SLEEP,
            Err(error) => {
                tracing::error!(%error, "failed to read the next scheduled run");
                RETRY_SLEEP
            }
        };
        tokio::select! {
            () = shutdown.cancelled() => return,
            () = tokio::time::sleep(sleep.max(Duration::from_millis(200))) => {}
        }
    }
}

async fn next_due(state: &ServersState) -> Result<Option<OffsetDateTime>, sqlx::Error> {
    let mut conn = state.pool.acquire().await?;
    ScheduleRepository::next_due(&mut conn).await
}
