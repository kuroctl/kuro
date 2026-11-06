//! Tâches de fond (ADR-0062), onglet « Tâches » des Réglages : lecture par
//! `system.settings.read`, « Lancer maintenant » et « Simulation » par `system.settings.update`.

use axum::Router;
use axum::extract::{FromRef, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use serde::Deserialize;
use utoipa::{OpenApi, ToSchema};

use super::{TaskStatus, Tasks, TasksError};
use crate::error_code::ErrorCode;
use crate::http::error::{ApiError, Problem, internal_error};
use crate::http::extract::{Json, Path};
use crate::iam::application::IamState;
use crate::iam::domain::Permission;
use crate::iam::http::AdminSession;

#[derive(OpenApi)]
#[openapi(
    paths(list_tasks, update_task, run_task),
    components(schemas(TaskStatus, TaskUpdateRequest)),
    tags((name = "admin-tasks", description = "Tâches de fond (ADR-0062)"))
)]
pub struct TasksApi;

pub fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    IamState: FromRef<S>,
    Tasks: FromRef<S>,
{
    Router::new()
        .route("/settings/tasks", get(list_tasks))
        .route("/settings/tasks/{name}", axum::routing::put(update_task))
        .route("/settings/tasks/{name}/run", post(run_task))
}

impl From<TasksError> for ApiError {
    fn from(error: TasksError) -> Self {
        match &error {
            TasksError::NotFound => Problem::new(StatusCode::NOT_FOUND, ErrorCode::NotFound).into(),
            TasksError::Database(_) | TasksError::Detached => {
                internal_error(&error, "task request failed").into()
            }
        }
    }
}

/// Tâches de fond, réglage et dernier passage. Permission : `system.settings.read`.
#[utoipa::path(get, path = "/api/admin/v1/settings/tasks", tag = "admin-tasks", security(("bearer" = [])),
    responses((status = 200, body = Vec<TaskStatus>), (status = 403, body = Problem)))]
pub async fn list_tasks(
    State(iam): State<IamState>,
    State(tasks): State<Tasks>,
    session: AdminSession,
) -> Result<Json<Vec<TaskStatus>>, ApiError> {
    session
        .require(&iam, Permission::SystemSettingsRead)
        .await?;
    Ok(Json(tasks.list().await?))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TaskUpdateRequest {
    /// Simulation : la tâche rapporte ce qu'elle ferait, sans agir.
    pub dry_run: bool,
}

/// Règle la simulation d'une tâche. Permission : `system.settings.update`.
#[utoipa::path(put, path = "/api/admin/v1/settings/tasks/{name}", tag = "admin-tasks", security(("bearer" = [])),
    params(("name" = String, Path)), request_body = TaskUpdateRequest,
    responses((status = 200, body = TaskStatus), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn update_task(
    State(iam): State<IamState>,
    State(tasks): State<Tasks>,
    session: AdminSession,
    Path(name): Path<String>,
    Json(request): Json<TaskUpdateRequest>,
) -> Result<Json<TaskStatus>, ApiError> {
    session
        .require_elevated(&iam, Permission::SystemSettingsUpdate)
        .await?;
    Ok(Json(
        tasks
            .set_dry_run(&session.context, &name, request.dry_run)
            .await?,
    ))
}

/// Avance le prochain passage (« Lancer maintenant ») ; il se fait en tâche de fond.
/// Permission : `system.settings.update`.
#[utoipa::path(post, path = "/api/admin/v1/settings/tasks/{name}/run", tag = "admin-tasks", security(("bearer" = [])),
    params(("name" = String, Path)),
    responses((status = 202), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn run_task(
    State(iam): State<IamState>,
    State(tasks): State<Tasks>,
    session: AdminSession,
    Path(name): Path<String>,
) -> Result<StatusCode, ApiError> {
    session
        .require(&iam, Permission::SystemSettingsUpdate)
        .await?;
    tasks.trigger(&name).await?;
    Ok(StatusCode::ACCEPTED)
}
