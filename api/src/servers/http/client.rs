//! Espace client (ADR-0051) : le propriétaire d'un serveur, identifié par le fournisseur d'une
//! extension, agit sur **ses** serveurs, montés sous `/api/client/v1`. Mêmes cas d'usage que
//! l'espace admin, autre garde : `OwnerSession` au lieu d'`AdminSession`.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use kuro_protocol::api::server::PowerAction;
use serde::Serialize;
use serde_json::json;
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

use super::servers::{PowerActionDto, PowerRequest, ServerAllocationDto, ServerPrefabDto};
use crate::audit::domain::{AuditAction, AuditEvent};
use crate::http::error::{ApiError, ErrorCode, Problem};
use crate::iam::http::OwnerSession;
use crate::servers::application::{self as servers, ServerView, ServersError, ServersState};
use crate::servers::domain::ServerState;

/// Serveur vu par son propriétaire : ce qu'il faut pour le reconnaître et le rejoindre, sans
/// nœud, sans réglages ni cause technique d'échec.
#[derive(Debug, Serialize, ToSchema)]
pub struct ClientServerDto {
    pub id: Uuid,
    pub name: String,
    pub prefab: ServerPrefabDto,
    pub state: ServerState,
    /// Suspendu : ni démarrage, ni fichiers, ni sauvegarde.
    pub suspended: bool,
    pub memory_mb: i32,
    /// 100 = un cœur.
    pub cpu_percent: i32,
    pub disk_mb: i32,
    pub backup_limit: i32,
    pub allocations: Vec<ServerAllocationDto>,
    /// Nom dans la zone DNS (`survie` pour `survie.<domaine>`).
    pub dns_label: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
}

fn dto(view: ServerView) -> ClientServerDto {
    let server = view.server;
    ClientServerDto {
        id: server.id,
        name: server.name,
        prefab: ServerPrefabDto {
            id: server.prefab_id,
            version: server.prefab_version,
        },
        state: server.state.parse().unwrap_or(ServerState::Failed),
        suspended: server.suspended_at.is_some(),
        memory_mb: server.memory_mb,
        cpu_percent: server.cpu_percent,
        disk_mb: server.disk_mb,
        backup_limit: server.backup_limit,
        allocations: view
            .allocations
            .into_iter()
            .map(|(name, ip, port)| ServerAllocationDto { name, ip, port })
            .collect(),
        dns_label: server.dns_label,
        created_at: server.created_at,
    }
}

/// Serveur de la session, sinon `404` : un serveur d'un autre propriétaire est indiscernable d'un
/// serveur inexistant. La tentative est auditée (`access.denied`, raison `not_owner`).
async fn owned(
    state: &ServersState,
    session: &OwnerSession,
    id: Uuid,
) -> Result<ServerView, ApiError> {
    let not_found = || ApiError::from(Problem::new(StatusCode::NOT_FOUND, ErrorCode::NotFound));
    let view = match servers::get(state, id).await {
        Ok(view) => view,
        Err(ServersError::NotFound) => return Err(not_found()),
        Err(error) => return Err(error.into()),
    };
    if session.owns(view.server.owner_ref.as_deref()) {
        return Ok(view);
    }
    let event = AuditEvent::denied(AuditAction::AccessDenied)
        .target("server", id)
        .details(json!({ "reason": "not_owner" }));
    if let Err(error) =
        crate::audit::application::record(&state.pool, &session.context, event).await
    {
        tracing::error!(%error, "failed to record access denial");
    }
    Err(not_found())
}

/// Serveurs du propriétaire, par nom.
#[utoipa::path(get, path = "/api/client/v1/servers", tag = "client-servers", security(("bearer" = [])),
    responses((status = 200, body = [ClientServerDto]), (status = 401, body = Problem)))]
pub async fn list(
    State(state): State<ServersState>,
    session: OwnerSession,
) -> Result<Json<Vec<ClientServerDto>>, ApiError> {
    let views = servers::list(&state, Some(session.owner.as_str()), None, 0).await?;
    Ok(Json(views.into_iter().map(dto).collect()))
}

/// Un serveur du propriétaire.
#[utoipa::path(get, path = "/api/client/v1/servers/{id}", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = ClientServerDto), (status = 401, body = Problem), (status = 404, body = Problem)))]
pub async fn get(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path(id): Path<Uuid>,
) -> Result<Json<ClientServerDto>, ApiError> {
    Ok(Json(dto(owned(&state, &session, id).await?)))
}

/// Démarre, arrête, redémarre ou tue un serveur du propriétaire. Refusé sur un serveur suspendu,
/// quelle que soit l'action.
#[utoipa::path(post, path = "/api/client/v1/servers/{id}/power", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = PowerRequest,
    responses((status = 202, description = "Action transmise au nœud"), (status = 401, body = Problem),
        (status = 404, body = Problem), (status = 409, description = "Serveur suspendu, ou pas prêt", body = Problem),
        (status = 502, body = Problem)))]
pub async fn power(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path(id): Path<Uuid>,
    Json(request): Json<PowerRequest>,
) -> Result<StatusCode, ApiError> {
    let view = owned(&state, &session, id).await?;
    if view.server.suspended_at.is_some() {
        return Err(ServersError::Conflict(ErrorCode::ServerSuspended).into());
    }
    let action = match request.action {
        PowerActionDto::Start => PowerAction::Start,
        PowerActionDto::Stop => PowerAction::Stop,
        PowerActionDto::Restart => PowerAction::Restart,
        PowerActionDto::Kill => PowerAction::Kill,
    };
    servers::power(&state, &session.context, id, action).await?;
    Ok(StatusCode::ACCEPTED)
}
