//! Espace client (ADR-0051) : le propriétaire d'un serveur, identifié par le fournisseur d'une
//! extension, agit sur **ses** serveurs, montés sous `/api/client/v1`. Mêmes cas d'usage que
//! l'espace admin, autre garde : `OwnerSession` au lieu d'`AdminSession`. Console et métriques
//! (ADR-0054) : mêmes tickets et même WebSocket que l'admin, le ticket désigne le propriétaire.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use kuro_protocol::api::server::PowerAction;
use serde::Serialize;
use serde_json::json;
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

use super::console::{CommandRequest, ConsoleTicketDto};
use super::metrics::{MetricsDto, MetricsQuery};
use super::servers::{PowerActionDto, PowerRequest, ServerAllocationDto, ServerPrefabDto};
use crate::audit::domain::{AuditAction, AuditEvent};
use crate::http::error::{ApiError, ErrorCode, Problem};
use crate::iam::http::OwnerSession;
use crate::servers::application::tickets::TicketHolder;
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

/// Serveur de la session (le sien, ou un dont elle est membre, ADR-0060), sinon `404` : un serveur
/// d'un autre propriétaire est indiscernable d'un serveur inexistant. La tentative est auditée
/// (`access.denied`, raison `not_owner`).
pub(super) async fn owned(
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
    if session.owns(view.server.owner_ref.as_deref())
        || servers::is_member(state, id, session.owner.as_str()).await?
    {
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

/// Serveurs du propriétaire et ceux dont il est membre, par nom.
#[utoipa::path(get, path = "/api/client/v1/servers", tag = "client-servers", security(("bearer" = [])),
    responses((status = 200, body = [ClientServerDto]), (status = 401, body = Problem)))]
pub async fn list(
    State(state): State<ServersState>,
    session: OwnerSession,
) -> Result<Json<Vec<ClientServerDto>>, ApiError> {
    let views = servers::of_client(&state, session.owner.as_str()).await?;
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

/// Ticket à usage unique (30 s) pour ouvrir la console d'un serveur du propriétaire (ADR-0054).
/// Refusé sur un serveur suspendu.
#[utoipa::path(post, path = "/api/client/v1/servers/{id}/console/ticket", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 201, body = ConsoleTicketDto), (status = 401, body = Problem), (status = 404, body = Problem),
        (status = 409, description = "Serveur suspendu, ou console tenue par une autre instance", body = Problem)))]
pub async fn console_ticket(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<ConsoleTicketDto>), ApiError> {
    let view = owned(&state, &session, id).await?;
    if view.server.suspended_at.is_some() {
        return Err(ServersError::Conflict(ErrorCode::ServerSuspended).into());
    }
    let holder = TicketHolder::Owner(session.owner.as_str().to_owned());
    let ticket = servers::console_ticket(&state, &session.context, id, &holder).await?;
    Ok((
        StatusCode::CREATED,
        Json(ConsoleTicketDto {
            ticket,
            expires_in_secs: servers::TICKET_TTL.as_secs(),
        }),
    ))
}

/// Envoie une ligne à la console d'un serveur du propriétaire (gardée dans l'audit).
#[utoipa::path(post, path = "/api/client/v1/servers/{id}/command", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = CommandRequest,
    responses((status = 202), (status = 401, body = Problem), (status = 404, body = Problem),
        (status = 409, description = "Serveur suspendu ou arrêté", body = Problem), (status = 422, body = Problem),
        (status = 502, body = Problem)))]
pub async fn command(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path(id): Path<Uuid>,
    Json(request): Json<CommandRequest>,
) -> Result<StatusCode, ApiError> {
    owned(&state, &session, id).await?;
    servers::command(&state, &session.context, id, &request.line).await?;
    Ok(StatusCode::ACCEPTED)
}

/// Historique CPU, mémoire et réseau d'un serveur du propriétaire : `24h` (par défaut), `7d`,
/// `90d`.
#[utoipa::path(get, path = "/api/client/v1/servers/{id}/metrics", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), MetricsQuery),
    responses((status = 200, body = MetricsDto), (status = 401, body = Problem), (status = 404, body = Problem),
        (status = 422, description = "Plage inconnue", body = Problem)))]
pub async fn metrics(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path(id): Path<Uuid>,
    Query(query): Query<MetricsQuery>,
) -> Result<Json<MetricsDto>, ApiError> {
    owned(&state, &session, id).await?;
    Ok(Json(
        super::metrics::load(&state, id, query.range.as_deref()).await?,
    ))
}

#[derive(Debug, serde::Deserialize, ToSchema)]
pub struct RenameRequest {
    /// Nom affiché, 64 caractères au plus ; le nom DNS du serveur ne change pas.
    #[schema(example = "Survie entre potes")]
    pub name: String,
}

/// Renomme un serveur du propriétaire (ADR-0058).
#[utoipa::path(patch, path = "/api/client/v1/servers/{id}", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = RenameRequest,
    responses((status = 200, body = ClientServerDto), (status = 401, body = Problem), (status = 404, body = Problem),
        (status = 422, body = Problem)))]
pub async fn rename(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path(id): Path<Uuid>,
    Json(request): Json<RenameRequest>,
) -> Result<Json<ClientServerDto>, ApiError> {
    owned(&state, &session, id).await?;
    Ok(Json(dto(servers::rename(
        &state,
        &session.context,
        id,
        &request.name,
    )
    .await?)))
}
