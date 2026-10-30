//! Console d'un serveur (ADR-0014) : ticket, commande, et WebSocket navigateur en lecture seule.

use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{FromRequestParts, State};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::http::context::request_context;
use crate::http::error::{ApiError, Problem};
use crate::http::extract::{Json, Path};
use crate::http::origin::CheckedOrigin;
use crate::iam::application::IamState;
use crate::iam::domain::Permission;
use crate::iam::http::AdminSession;
use crate::nodes::domain::NodeId;
use crate::servers::application::{self as servers, ConsoleEvent, ServersState, TICKET_TTL};
use crate::servers::domain::ServerState;

/// Délai pour présenter le ticket après l'ouverture du WebSocket.
const AUTH_TIMEOUT: Duration = Duration::from_secs(5);
/// Ticket absent, invalide, expiré ou d'un autre serveur.
const CLOSE_UNAUTHORIZED: u16 = 4401;
/// Serveur supprimé entre le ticket et la connexion.
const CLOSE_NOT_FOUND: u16 = 4404;
/// Durée maximale atteinte : le panel se reconnecte avec un nouveau ticket.
const CLOSE_EXPIRED: u16 = 4408;

#[derive(Debug, Serialize, ToSchema)]
pub struct ConsoleTicketDto {
    /// À envoyer comme premier message du WebSocket, jamais dans l'URL.
    pub ticket: String,
    pub expires_in_secs: u64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CommandRequest {
    /// Une ligne, sans caractère de contrôle, 1 024 caractères au plus.
    #[schema(example = "say Bonjour")]
    pub line: String,
}

/// Ticket à usage unique (30 s) pour ouvrir le WebSocket de console.
/// Permission : `servers.console.read`.
#[utoipa::path(post, path = "/api/admin/v1/servers/{id}/console/ticket", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 201, body = ConsoleTicketDto), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn ticket(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<ConsoleTicketDto>), ApiError> {
    session
        .require(&iam, Permission::ServersConsoleRead)
        .await?;
    let ticket =
        servers::console_ticket(&state, &session.context, id, session.principal.user_id).await?;
    Ok((
        StatusCode::CREATED,
        Json(ConsoleTicketDto {
            ticket,
            expires_in_secs: TICKET_TTL.as_secs(),
        }),
    ))
}

/// Envoie une ligne à la console du serveur (gardée dans l'audit, tronquée à 256 caractères).
/// Permission : `servers.console.write`.
#[utoipa::path(post, path = "/api/admin/v1/servers/{id}/command", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = CommandRequest,
    responses((status = 202), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn command(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Json(request): Json<CommandRequest>,
) -> Result<StatusCode, ApiError> {
    session
        .require(&iam, Permission::ServersConsoleWrite)
        .await?;
    servers::command(&state, &session.context, id, &request.line).await?;
    Ok(StatusCode::ACCEPTED)
}

pub(super) async fn denied(
    state: &ServersState,
    context: &AuditContext,
    details: serde_json::Value,
) {
    let event = AuditEvent::denied(AuditAction::AccessDenied).details(details);
    if let Err(error) = crate::audit::application::record(&state.pool, context, event).await {
        tracing::error!(%error, "failed to record console access denial");
    }
}

/// WebSocket de console, en lecture seule. Premier message attendu :
/// `{ "type": "auth", "ticket": "…" }`. Ensuite : `lines`, `stats`, `state`.
pub async fn socket(
    State(state): State<ServersState>,
    _origin: CheckedOrigin,
    Path(id): Path<Uuid>,
    parts: RequestContext,
    upgrade: WebSocketUpgrade,
) -> Response {
    let context = parts.0;
    upgrade
        .max_message_size(4096)
        .on_upgrade(move |socket| async move {
            run(socket, state, id, context).await;
        })
        .into_response()
}

/// Contexte d'audit de la requête d'ouverture.
pub struct RequestContext(pub AuditContext);

impl<S: Send + Sync> FromRequestParts<S> for RequestContext {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(request_context(parts)))
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    Auth { ticket: String },
}

async fn run(mut socket: WebSocket, state: ServersState, id: Uuid, context: AuditContext) {
    let Some(admin_id) = authenticate(&mut socket, &state, id).await else {
        denied(
            &state,
            &context,
            json!({ "reason": "console_ticket", "server_id": id }),
        )
        .await;
        close(&mut socket, CLOSE_UNAUTHORIZED, "invalid ticket").await;
        return;
    };
    let server = match servers::get(&state, id).await {
        Ok(view) => view.server,
        Err(_) => {
            close(&mut socket, CLOSE_NOT_FOUND, "server not found").await;
            return;
        }
    };
    tracing::debug!(server_id = %id, %admin_id, "console opened");

    let mut viewer = state.console.join(id, NodeId::from(server.node_id));
    let initial = server
        .state
        .parse::<ServerState>()
        .ok()
        .map(|s| ConsoleEvent::State {
            state: s.as_str(),
            reason: server.state_reason.clone(),
        });
    let history = std::mem::take(&mut viewer.history);
    let opening = initial
        .into_iter()
        .chain((!history.is_empty()).then_some(ConsoleEvent::Lines {
            lines: history,
            dropped: 0,
        }));
    for event in opening {
        if send(&mut socket, &event).await.is_err() {
            return;
        }
    }

    let deadline = tokio::time::sleep(state.console_access.max_session);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            () = &mut deadline => {
                close(&mut socket, CLOSE_EXPIRED, "session expired").await;
                return;
            }
            event = viewer.events.recv() => {
                let event = match event {
                    Ok(event) => event,
                    Err(RecvError::Lagged(skipped)) => ConsoleEvent::Lines { lines: Vec::new(), dropped: skipped },
                    Err(RecvError::Closed) => return,
                };
                if send(&mut socket, &event).await.is_err() {
                    return;
                }
            }
            frame = socket.next() => match frame {
                // Lecture seule : les messages du navigateur sont ignorés.
                Some(Ok(Message::Text(_) | Message::Binary(_) | Message::Ping(_) | Message::Pong(_))) => {}
                Some(Ok(Message::Close(_)) | Err(_)) | None => return,
            },
        }
    }
}

/// Premier message : le ticket. `None` s'il manque, est invalide ou arrive trop tard.
async fn authenticate(socket: &mut WebSocket, state: &ServersState, id: Uuid) -> Option<Uuid> {
    let first = tokio::time::timeout(AUTH_TIMEOUT, async {
        loop {
            match socket.next().await? {
                Ok(Message::Text(text)) => return Some(text),
                Ok(Message::Ping(_) | Message::Pong(_)) => {}
                _ => return None,
            }
        }
    })
    .await
    .ok()??;
    let ClientMessage::Auth { ticket } = serde_json::from_str(first.as_str()).ok()?;
    match servers::tickets::redeem_console(&state.pool, &ticket, id).await {
        Ok(admin) => admin,
        Err(error) => {
            tracing::error!(%error, "failed to redeem a console ticket");
            None
        }
    }
}

async fn send(socket: &mut WebSocket, event: &ConsoleEvent) -> Result<(), ()> {
    let text = serde_json::to_string(event).map_err(|_| ())?;
    socket
        .send(Message::Text(text.into()))
        .await
        .map_err(|_| ())
}

async fn close(socket: &mut WebSocket, code: u16, reason: &'static str) {
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        })))
        .await;
}
