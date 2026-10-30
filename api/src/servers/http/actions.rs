//! Actions d'un prefab (ADR-0039) : `POST /servers/{id}/actions/{key}`.

use axum::extract::State;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::http::error::{ApiError, Problem};
use crate::http::extract::{Json, Path};
use crate::iam::application::IamState;
use crate::iam::domain::Permission;
use crate::iam::http::AdminSession;
use crate::servers::application::{self as servers, ServersState};

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ActionRequest {
    /// Paramètres déclarés par l'action ; un paramètre absent prend sa valeur par défaut.
    #[serde(default)]
    #[schema(value_type = Object)]
    pub params: Map<String, Value>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ActionResultDto {
    pub exit_code: i64,
    /// Sortie du processus (stdout et stderr), tronquée par le nœud à 4 Kio.
    pub output: String,
    pub truncated: bool,
}

/// Lance une action déclarée par le prefab, dans le conteneur de son service en marche.
/// Permission : `servers.actions.run`.
#[utoipa::path(post, path = "/api/admin/v1/servers/{id}/actions/{key}", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("key" = String, Path, description = "Clé de l'action dans le prefab")),
    request_body = ActionRequest,
    responses((status = 200, body = ActionResultDto), (status = 403, body = Problem),
        (status = 404, description = "Serveur ou action inconnus", body = Problem),
        (status = 409, description = "Serveur arrêté, suspendu, occupé, ou daemon trop ancien", body = Problem),
        (status = 422, description = "Paramètre refusé", body = Problem), (status = 502, body = Problem)))]
pub async fn run(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path((id, key)): Path<(Uuid, String)>,
    Json(request): Json<ActionRequest>,
) -> Result<Json<ActionResultDto>, ApiError> {
    session.require(&iam, Permission::ServersActionsRun).await?;
    let outcome =
        servers::actions::run(&state, &session.context, id, &key, &request.params).await?;
    Ok(Json(ActionResultDto {
        exit_code: outcome.exit_code,
        output: outcome.output,
        truncated: outcome.truncated,
    }))
}
