//! Tâches de fond de Kuro et des extensions (ADR-0062) : une boucle par tâche, un passage à la
//! fois sur l'ensemble des instances (verrou consultatif), déclenchement sur demande relayé par
//! le bus, simulation réglée dans le panel, dernier passage noté dans `background_tasks`.

pub mod http;

use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use sqlx::PgPool;
use time::OffsetDateTime;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::cluster::Bus;

/// Canal du bus : le payload est le nom de la tâche à avancer.
pub const CHANNEL: &str = "kuro_tasks";
/// Plusieurs déclenchements rapprochés n'en font qu'un.
const DEBOUNCE: Duration = Duration::from_millis(500);
/// Décalage aléatoire maximal d'un passage périodique (instances qui s'éveillent ensemble).
const JITTER_MS: u128 = 5_000;

/// Déclaration d'une tâche : nom (préfixé par son module ou son extension) et période.
#[derive(Debug, Clone, Copy)]
pub struct TaskDef {
    /// `sweep` ; nom complet `machines.sweep`, `cloud.renewals`.
    pub name: &'static str,
    pub period: Duration,
    /// Simulation au premier lancement : une tâche destructive commence par observer.
    pub dry_run_by_default: bool,
}

/// Ce qu'un passage reçoit.
#[derive(Debug, Clone)]
pub struct TaskContext {
    pub pool: PgPool,
    /// État de l'application : cas d'usage de Kuro (serveurs, machines, e-mails, IAM, tâches…).
    pub app: crate::state::AppState,
    /// Section `[extensions.<nom>]` de la configuration, pour une tâche d'extension.
    pub config: Option<Value>,
    pub now: OffsetDateTime,
    /// Calculer et rapporter ce qui serait fait, sans agir (Kuro ne l'interprète pas).
    pub dry_run: bool,
    /// Arrêt de Kuro : un passage interrompu est repris au suivant.
    pub shutdown: CancellationToken,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct TaskError(pub String);

/// Un passage : son rapport (compteurs, JSON libre).
pub type TaskFn =
    Arc<dyn Fn(TaskContext) -> BoxFuture<'static, Result<Value, TaskError>> + Send + Sync>;

/// Tâche prête à tourner.
#[derive(Clone)]
pub struct Task {
    pub name: String,
    pub def: TaskDef,
    pub config: Option<Value>,
    pub run: TaskFn,
}

impl Task {
    pub fn new(
        prefix: &str,
        def: TaskDef,
        config: Option<Value>,
        run: impl Fn(TaskContext) -> BoxFuture<'static, Result<Value, TaskError>>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            name: format!("{prefix}.{}", def.name),
            def,
            config,
            run: Arc::new(run),
        }
    }
}

impl std::fmt::Debug for Task {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Task")
            .field("name", &self.name)
            .field("def", &self.def)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TasksError {
    #[error("unknown task")]
    NotFound,
    /// L'état de l'application n'est pas encore rattaché (`extension::start`).
    #[error("application state not attached")]
    Detached,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// État d'une tâche, pour le panel.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct TaskStatus {
    pub name: String,
    pub period_secs: u64,
    pub dry_run: bool,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub started_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub finished_at: Option<OffsetDateTime>,
    pub duration_ms: Option<i64>,
    /// `ok` ou `error`.
    pub outcome: Option<String>,
    pub error: Option<String>,
    /// Rapport du dernier passage.
    #[schema(value_type = Option<Object>)]
    pub report: Option<Value>,
    /// Le dernier passage était une simulation.
    pub last_dry_run: Option<bool>,
    /// Un passage a commencé et n'est pas fini.
    pub running: bool,
}

/// Registre des tâches de ce processus : leurs réveils et leur exécution.
#[derive(Clone)]
pub struct Tasks {
    pool: PgPool,
    tasks: Arc<Vec<Task>>,
    wakes: Arc<HashMap<String, Arc<Notify>>>,
    /// État de l'application, rattaché une fois construit (il contient ce registre : référence
    /// faible).
    app: Arc<OnceLock<crate::state::AppHandle>>,
}

impl std::fmt::Debug for Tasks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tasks")
            .field("tasks", &self.tasks)
            .finish_non_exhaustive()
    }
}

impl Tasks {
    pub fn new(pool: PgPool, tasks: Vec<Task>) -> Self {
        let wakes = tasks
            .iter()
            .map(|task| (task.name.clone(), Arc::new(Notify::new())))
            .collect();
        Self {
            pool,
            tasks: Arc::new(tasks),
            wakes: Arc::new(wakes),
            app: Arc::default(),
        }
    }

    /// Rattache l'état de l'application, passé à chaque passage ; un second rattachement est
    /// ignoré. Fait par [`crate::extension::start`].
    pub fn attach(&self, app: crate::state::AppHandle) {
        let _ = self.app.set(app);
    }

    fn task(&self, name: &str) -> Option<&Task> {
        self.tasks.iter().find(|task| task.name == name)
    }

    /// Crée la ligne de chaque tâche (simulation par défaut de sa déclaration).
    pub async fn register(&self) -> Result<(), sqlx::Error> {
        for task in self.tasks.iter() {
            sqlx::query!(
                "INSERT INTO background_tasks (name, dry_run) VALUES ($1, $2)
                 ON CONFLICT (name) DO NOTHING",
                task.name,
                task.def.dry_run_by_default,
            )
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    /// Avance le prochain passage de `name`, sur toutes les instances (bus).
    pub async fn trigger(&self, name: &str) -> Result<(), TasksError> {
        if self.task(name).is_none() {
            return Err(TasksError::NotFound);
        }
        self.wake_local(name);
        Bus::notify(&self.pool, CHANNEL, name).await?;
        Ok(())
    }

    /// Réveil reçu par le bus (ou local).
    pub fn wake_local(&self, name: &str) {
        if let Some(wake) = self.wakes.get(name) {
            wake.notify_one();
        }
    }

    pub async fn list(&self) -> Result<Vec<TaskStatus>, TasksError> {
        let rows = sqlx::query!(
            "SELECT name, dry_run, started_at, finished_at, duration_ms, outcome, error, report,
                    last_dry_run
               FROM background_tasks ORDER BY name"
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(self
            .tasks
            .iter()
            .map(|task| {
                let row = rows.iter().find(|row| row.name == task.name);
                let started_at = row.and_then(|row| row.started_at);
                let finished_at = row.and_then(|row| row.finished_at);
                TaskStatus {
                    name: task.name.clone(),
                    period_secs: task.def.period.as_secs(),
                    dry_run: row.map_or(task.def.dry_run_by_default, |row| row.dry_run),
                    started_at,
                    finished_at,
                    duration_ms: row.and_then(|row| row.duration_ms),
                    outcome: row.and_then(|row| row.outcome.clone()),
                    error: row.and_then(|row| row.error.clone()),
                    report: row.and_then(|row| row.report.clone()),
                    last_dry_run: row.and_then(|row| row.last_dry_run),
                    running: match (started_at, finished_at) {
                        (Some(started), Some(finished)) => started > finished,
                        (Some(_), None) => true,
                        _ => false,
                    },
                }
            })
            .collect())
    }

    pub async fn status(&self, name: &str) -> Result<TaskStatus, TasksError> {
        self.list()
            .await?
            .into_iter()
            .find(|task| task.name == name)
            .ok_or(TasksError::NotFound)
    }

    /// Réglage « Simulation » d'une tâche, audité.
    pub async fn set_dry_run(
        &self,
        context: &AuditContext,
        name: &str,
        dry_run: bool,
    ) -> Result<TaskStatus, TasksError> {
        if self.task(name).is_none() {
            return Err(TasksError::NotFound);
        }
        let mut tx = AuditedTx::begin(&self.pool, context).await?;
        sqlx::query!(
            "INSERT INTO background_tasks (name, dry_run) VALUES ($1, $2)
             ON CONFLICT (name) DO UPDATE SET dry_run = EXCLUDED.dry_run",
            name,
            dry_run,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit(
            AuditEvent::success(AuditAction::TaskUpdated)
                .target("task", name)
                .details(json!({ "dry_run": dry_run })),
        )
        .await?;
        self.status(name).await
    }

    /// Simulation réglée pour `name` ; sans réglage, celle de sa déclaration (vraie pour une
    /// tâche inconnue : rien de destructif).
    pub async fn dry_run(&self, name: &str) -> Result<bool, sqlx::Error> {
        let dry_run =
            sqlx::query_scalar!("SELECT dry_run FROM background_tasks WHERE name = $1", name)
                .fetch_optional(&self.pool)
                .await?;
        let default = self
            .task(name)
            .is_none_or(|task| task.def.dry_run_by_default);
        Ok(dry_run.unwrap_or(default))
    }

    /// Un passage de `name` sous le verrou de la tâche ; `None` si une autre instance le tient.
    pub async fn pass(
        &self,
        name: &str,
        shutdown: &CancellationToken,
    ) -> Result<Option<TaskStatus>, TasksError> {
        let task = self.task(name).ok_or(TasksError::NotFound)?.clone();
        let key = format!("task:{name}");
        let mut lock = self.pool.acquire().await?;
        let locked: Option<bool> =
            sqlx::query_scalar!("SELECT pg_try_advisory_lock(hashtext($1)::bigint)", key)
                .fetch_one(&mut *lock)
                .await?;
        if locked != Some(true) {
            return Ok(None);
        }
        let outcome = self.run_locked(&task, shutdown).await;
        if let Err(error) =
            sqlx::query_scalar!("SELECT pg_advisory_unlock(hashtext($1)::bigint)", key)
                .fetch_one(&mut *lock)
                .await
        {
            // Le verrou tombera avec la connexion ; d'ici là, les autres instances passent leur tour.
            tracing::warn!(%error, task = name, "failed to release the task lock");
        }
        outcome?;
        self.status(name).await.map(Some)
    }

    async fn run_locked(
        &self,
        task: &Task,
        shutdown: &CancellationToken,
    ) -> Result<(), TasksError> {
        let Some(app) = self.app.get().and_then(crate::state::AppHandle::get) else {
            return Err(TasksError::Detached);
        };
        let dry_run = self.dry_run(&task.name).await?;
        let now = OffsetDateTime::now_utc();
        sqlx::query!(
            "INSERT INTO background_tasks (name, dry_run, started_at) VALUES ($1, $2, $3)
             ON CONFLICT (name) DO UPDATE SET started_at = EXCLUDED.started_at",
            task.name,
            dry_run,
            now,
        )
        .execute(&self.pool)
        .await?;
        let context = TaskContext {
            pool: self.pool.clone(),
            app,
            config: task.config.clone(),
            now,
            dry_run,
            shutdown: shutdown.clone(),
        };
        let started = Instant::now();
        let run = AssertUnwindSafe((task.run)(context)).catch_unwind();
        let result = tokio::select! {
            () = shutdown.cancelled() => Err("interrupted by shutdown".to_owned()),
            result = run => match result {
                Ok(Ok(report)) => Ok(report),
                Ok(Err(error)) => Err(error.0),
                Err(_) => Err("task panicked".to_owned()),
            },
        };
        let duration = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
        let (outcome, error, report) = match result {
            Ok(report) => {
                tracing::info!(task = %task.name, dry_run, %report, "task pass finished");
                ("ok", None, Some(report))
            }
            Err(error) => {
                tracing::error!(task = %task.name, %error, "task pass failed");
                ("error", Some(error), None)
            }
        };
        sqlx::query!(
            "UPDATE background_tasks
                SET finished_at = $7, duration_ms = $2, outcome = $3, error = $4,
                    report = COALESCE($5, report), last_dry_run = $6
              WHERE name = $1",
            task.name,
            duration,
            outcome,
            error,
            report,
            dry_run,
            // Même horloge que le début du passage : un passage bref ne paraît jamais en cours.
            now + started.elapsed(),
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Une boucle par tâche, jusqu'à l'arrêt.
    pub async fn run_forever(self, shutdown: CancellationToken) {
        let loops = self.tasks.iter().map(|task| {
            let tasks = self.clone();
            let shutdown = shutdown.clone();
            let name = task.name.clone();
            let period = task.def.period;
            async move { tasks.run_loop(&name, period, shutdown).await }
        });
        futures_util::future::join_all(loops).await;
    }

    async fn run_loop(&self, name: &str, period: Duration, shutdown: CancellationToken) {
        let Some(wake) = self.wakes.get(name).cloned() else {
            return;
        };
        loop {
            let jitter = Duration::from_millis(
                u64::try_from(uuid::Uuid::new_v4().as_u128() % JITTER_MS).unwrap_or(0),
            );
            tokio::select! {
                () = shutdown.cancelled() => return,
                () = wake.notified() => tokio::time::sleep(DEBOUNCE).await,
                () = tokio::time::sleep(period + jitter) => {}
            }
            if let Err(error) = self.pass(name, &shutdown).await {
                tracing::error!(task = name, %error, "task pass not recorded");
            }
        }
    }
}
