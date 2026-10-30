//! Tâches planifiées stockées (ADR-0022) : déclencheur, étapes, exécutions.

use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::servers::domain::schedule::Step;

#[derive(Debug, Clone)]
pub struct ScheduleRecord {
    pub id: Uuid,
    pub server_id: Uuid,
    pub name: String,
    pub cron: String,
    pub timezone: String,
    pub enabled: bool,
    pub only_when_online: bool,
    pub next_run_at: Option<OffsetDateTime>,
    pub running_since: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Clone)]
pub struct StepRecord {
    pub schedule_id: Uuid,
    pub position: i32,
    pub action: String,
    pub payload: Option<String>,
    pub delay_secs: i32,
    pub continue_on_failure: bool,
}

#[derive(Debug, Clone)]
pub struct RunRecord {
    pub id: Uuid,
    pub schedule_id: Uuid,
    pub trigger: String,
    pub started_at: OffsetDateTime,
    pub finished_at: Option<OffsetDateTime>,
    pub outcome: String,
    pub failed_step: Option<i32>,
    pub reason: Option<String>,
}

/// Champs écrits à la création comme à la modification.
#[derive(Debug, Clone)]
pub struct ScheduleFields<'a> {
    pub name: &'a str,
    pub cron: &'a str,
    pub timezone: &'a str,
    pub enabled: bool,
    pub only_when_online: bool,
    pub next_run_at: Option<OffsetDateTime>,
}

#[derive(Debug)]
pub struct ScheduleRepository;

impl ScheduleRepository {
    pub async fn count_for_server(
        conn: &mut PgConnection,
        server_id: Uuid,
    ) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar!(
            r#"SELECT count(*) AS "count!" FROM server_schedules WHERE server_id = $1"#,
            server_id,
        )
        .fetch_one(conn)
        .await
    }

    /// Tâches du serveur, par ordre de création.
    pub async fn of_server(
        conn: &mut PgConnection,
        server_id: Uuid,
    ) -> Result<Vec<ScheduleRecord>, sqlx::Error> {
        sqlx::query_as!(
            ScheduleRecord,
            "SELECT id, server_id, name, cron, timezone, enabled, only_when_online, next_run_at,
                    running_since, created_at
             FROM server_schedules WHERE server_id = $1 ORDER BY created_at, id",
            server_id,
        )
        .fetch_all(conn)
        .await
    }

    pub async fn get(
        conn: &mut PgConnection,
        server_id: Uuid,
        id: Uuid,
    ) -> Result<Option<ScheduleRecord>, sqlx::Error> {
        sqlx::query_as!(
            ScheduleRecord,
            "SELECT id, server_id, name, cron, timezone, enabled, only_when_online, next_run_at,
                    running_since, created_at
             FROM server_schedules WHERE server_id = $1 AND id = $2",
            server_id,
            id,
        )
        .fetch_optional(conn)
        .await
    }

    /// Verrouille la tâche pour une modification ou un lancement manuel.
    pub async fn lock(
        conn: &mut PgConnection,
        server_id: Uuid,
        id: Uuid,
    ) -> Result<Option<ScheduleRecord>, sqlx::Error> {
        sqlx::query_as!(
            ScheduleRecord,
            "SELECT id, server_id, name, cron, timezone, enabled, only_when_online, next_run_at,
                    running_since, created_at
             FROM server_schedules WHERE server_id = $1 AND id = $2 FOR UPDATE",
            server_id,
            id,
        )
        .fetch_optional(conn)
        .await
    }

    /// Étapes de ces tâches, dans l'ordre.
    pub async fn steps(
        conn: &mut PgConnection,
        schedule_ids: &[Uuid],
    ) -> Result<Vec<StepRecord>, sqlx::Error> {
        sqlx::query_as!(
            StepRecord,
            "SELECT schedule_id, position, action, payload, delay_secs, continue_on_failure
             FROM server_schedule_steps WHERE schedule_id = ANY($1)
             ORDER BY schedule_id, position",
            schedule_ids,
        )
        .fetch_all(conn)
        .await
    }

    pub async fn insert(
        conn: &mut PgConnection,
        id: Uuid,
        server_id: Uuid,
        created_by: Option<Uuid>,
        fields: &ScheduleFields<'_>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO server_schedules
                 (id, server_id, name, cron, timezone, enabled, only_when_online, next_run_at,
                  created_by)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            id,
            server_id,
            fields.name,
            fields.cron,
            fields.timezone,
            fields.enabled,
            fields.only_when_online,
            fields.next_run_at,
            created_by,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn update(
        conn: &mut PgConnection,
        id: Uuid,
        fields: &ScheduleFields<'_>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE server_schedules
             SET name = $2, cron = $3, timezone = $4, enabled = $5, only_when_online = $6,
                 next_run_at = $7, updated_at = now()
             WHERE id = $1",
            id,
            fields.name,
            fields.cron,
            fields.timezone,
            fields.enabled,
            fields.only_when_online,
            fields.next_run_at,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Remplace toutes les étapes de la tâche.
    pub async fn replace_steps(
        conn: &mut PgConnection,
        id: Uuid,
        steps: &[Step],
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "DELETE FROM server_schedule_steps WHERE schedule_id = $1",
            id
        )
        .execute(&mut *conn)
        .await?;
        for (position, step) in steps.iter().enumerate() {
            let position = i32::try_from(position).unwrap_or(i32::MAX);
            sqlx::query!(
                "INSERT INTO server_schedule_steps
                     (schedule_id, position, action, payload, delay_secs, continue_on_failure)
                 VALUES ($1, $2, $3, $4, $5, $6)",
                id,
                position,
                step.action.kind(),
                step.action.payload(),
                step.delay_secs,
                step.continue_on_failure,
            )
            .execute(&mut *conn)
            .await?;
        }
        Ok(())
    }

    pub async fn delete(conn: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query!("DELETE FROM server_schedules WHERE id = $1", id)
            .execute(conn)
            .await?;
        Ok(())
    }

    /// Tâches actives dont l'échéance est passée, verrouillées sans attendre celles qu'une autre
    /// instance de l'API traite déjà.
    pub async fn lock_due(
        conn: &mut PgConnection,
        now: OffsetDateTime,
        limit: i64,
    ) -> Result<Vec<ScheduleRecord>, sqlx::Error> {
        sqlx::query_as!(
            ScheduleRecord,
            "SELECT id, server_id, name, cron, timezone, enabled, only_when_online, next_run_at,
                    running_since, created_at
             FROM server_schedules
             WHERE enabled AND next_run_at <= $1
             ORDER BY next_run_at
             LIMIT $2
             FOR UPDATE SKIP LOCKED",
            now,
            limit,
        )
        .fetch_all(conn)
        .await
    }

    /// Prochaine échéance parmi les tâches actives.
    pub async fn next_due(conn: &mut PgConnection) -> Result<Option<OffsetDateTime>, sqlx::Error> {
        sqlx::query_scalar!("SELECT min(next_run_at) FROM server_schedules WHERE enabled")
            .fetch_one(conn)
            .await
    }

    pub async fn set_next_run(
        conn: &mut PgConnection,
        id: Uuid,
        next_run_at: Option<OffsetDateTime>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE server_schedules SET next_run_at = $2 WHERE id = $1",
            id,
            next_run_at,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn set_running(
        conn: &mut PgConnection,
        id: Uuid,
        since: Option<OffsetDateTime>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE server_schedules SET running_since = $2 WHERE id = $1",
            id,
            since,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Ouvre une exécution (`running`) ou en note une sautée (`skipped`, avec sa raison).
    pub async fn insert_run(conn: &mut PgConnection, run: &RunRecord) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO server_schedule_runs
                 (id, schedule_id, trigger, started_at, finished_at, outcome, failed_step, reason)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            run.id,
            run.schedule_id,
            run.trigger,
            run.started_at,
            run.finished_at,
            run.outcome,
            run.failed_step,
            run.reason,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn finish_run(
        conn: &mut PgConnection,
        id: Uuid,
        outcome: &str,
        failed_step: Option<i32>,
        reason: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE server_schedule_runs
             SET finished_at = now(), outcome = $2, failed_step = $3, reason = $4
             WHERE id = $1",
            id,
            outcome,
            failed_step,
            reason,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Exécutions de la tâche, de la plus récente à la plus ancienne.
    pub async fn runs(
        conn: &mut PgConnection,
        schedule_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RunRecord>, sqlx::Error> {
        sqlx::query_as!(
            RunRecord,
            "SELECT id, schedule_id, trigger, started_at, finished_at, outcome, failed_step, reason
             FROM server_schedule_runs WHERE schedule_id = $1
             ORDER BY started_at DESC, id DESC LIMIT $2",
            schedule_id,
            limit,
        )
        .fetch_all(conn)
        .await
    }

    /// Dernière exécution de chaque tâche du serveur.
    pub async fn last_runs(
        conn: &mut PgConnection,
        server_id: Uuid,
    ) -> Result<Vec<RunRecord>, sqlx::Error> {
        sqlx::query_as!(
            RunRecord,
            "SELECT DISTINCT ON (r.schedule_id)
                    r.id, r.schedule_id, r.trigger, r.started_at, r.finished_at, r.outcome,
                    r.failed_step, r.reason
             FROM server_schedule_runs r
             JOIN server_schedules s ON s.id = r.schedule_id
             WHERE s.server_id = $1
             ORDER BY r.schedule_id, r.started_at DESC, r.id DESC",
            server_id,
        )
        .fetch_all(conn)
        .await
    }

    /// Ne garde que les `keep` exécutions les plus récentes de la tâche.
    pub async fn prune_runs(
        conn: &mut PgConnection,
        schedule_id: Uuid,
        keep: i64,
    ) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!(
            "DELETE FROM server_schedule_runs
             WHERE schedule_id = $1 AND id NOT IN (
                 SELECT id FROM server_schedule_runs WHERE schedule_id = $1
                 ORDER BY started_at DESC, id DESC LIMIT $2)",
            schedule_id,
            keep,
        )
        .execute(conn)
        .await?;
        Ok(result.rows_affected())
    }

    /// Au démarrage de l'API : les exécutions laissées en cours par un arrêt sont closes en échec
    /// et les tâches libérées.
    pub async fn recover(conn: &mut PgConnection) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!(
            "UPDATE server_schedule_runs
             SET outcome = 'failed', finished_at = now(), reason = 'interrupted'
             WHERE outcome = 'running'"
        )
        .execute(&mut *conn)
        .await?;
        sqlx::query!(
            "UPDATE server_schedules SET running_since = NULL WHERE running_since IS NOT NULL"
        )
        .execute(&mut *conn)
        .await?;
        Ok(result.rows_affected())
    }
}
