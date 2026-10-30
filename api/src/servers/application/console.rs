//! Console et stats en temps réel (ADR-0014) : spectateurs, abonnements des nœuds et commandes.
//! Les tickets du WebSocket navigateur sont dans [`super::tickets`].
//!
//! Le daemon ne lit la console d'un serveur que tant qu'un navigateur la regarde : le hub compte
//! les spectateurs et envoie `server.watch` au premier arrivé et au dernier parti, dans l'ordre.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use kuro_protocol::ApiMessage;
use kuro_protocol::api::server::{ServerCommand, ServerWatch};
use kuro_protocol::daemon::server::{CommandErrorCode, CommandResult, ServerStateReport};
use kuro_protocol::daemon::{ConsoleLine, ConsoleOutput, ServerStats};
use kuro_protocol::limits::{CONSOLE_HISTORY_LINES, is_valid_command};
use serde::Serialize;
use serde_json::json;
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

use super::deploy::accepted;
use super::{ServersError, ServersState, get};
use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::error_code::{ErrorCode, FieldError};
use crate::nodes::domain::NodeId;
use crate::servers::domain::{NodeCommands, ServerState};

/// Messages en attente par spectateur : au-delà, un spectateur lent saute des messages.
const VIEWER_QUEUE: usize = 256;
/// Caractères d'une commande gardés dans l'audit.
const AUDITED_COMMAND_CHARS: usize = 256;

/// Règles d'ouverture du WebSocket navigateur.
#[derive(Debug, Clone)]
pub struct ConsoleAccess {
    /// Au-delà, la connexion est fermée ; le panel en rouvre une avec un nouveau ticket, ce qui
    /// revérifie les droits. Égale à la durée d'un jeton d'accès.
    pub max_session: Duration,
}

/// Message envoyé au navigateur sur le WebSocket de console.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConsoleEvent {
    Lines {
        lines: Vec<ConsoleLine>,
        /// Lignes perdues (nœud ou spectateur trop lent).
        #[serde(skip_serializing_if = "is_zero")]
        dropped: u64,
    },
    Stats {
        cpu_millis: u64,
        memory_bytes: u64,
        memory_limit_bytes: u64,
        network_rx_bytes: u64,
        network_tx_bytes: u64,
    },
    State {
        state: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

/// Spectateurs et abonnements, partagés entre le REST, le WebSocket navigateur et le lien daemon.
#[derive(Debug, Clone)]
pub struct ConsoleHub {
    inner: Arc<HubInner>,
}

#[derive(Debug)]
struct HubInner {
    watched: Mutex<HashMap<Uuid, Watched>>,
    /// `server.watch` à envoyer, dans l'ordre des arrivées et départs.
    watches: mpsc::UnboundedSender<(NodeId, Uuid, bool)>,
}

#[derive(Debug)]
struct Watched {
    node: NodeId,
    viewers: usize,
    history: VecDeque<ConsoleLine>,
    events: broadcast::Sender<ConsoleEvent>,
}

impl Watched {
    /// Ligne antérieure à la dernière connue, ou identique à une ligne de la même milliseconde.
    fn knows(&self, line: &ConsoleLine) -> bool {
        let Some(last) = self.history.back() else {
            return false;
        };
        line.at_ms < last.at_ms
            || (line.at_ms == last.at_ms
                && self
                    .history
                    .iter()
                    .rev()
                    .take_while(|known| known.at_ms == line.at_ms)
                    .any(|known| known.text == line.text))
    }
}

/// Spectateur inscrit : l'historique au moment de l'inscription, puis le direct. Sa
/// destruction le désinscrit.
#[derive(Debug)]
pub struct Viewer {
    pub history: Vec<ConsoleLine>,
    pub events: broadcast::Receiver<ConsoleEvent>,
    hub: ConsoleHub,
    server_id: Uuid,
}

impl Drop for Viewer {
    fn drop(&mut self) {
        self.hub.leave(self.server_id);
    }
}

impl ConsoleHub {
    /// Démarre l'envoi ordonné des `server.watch` (requiert un runtime tokio).
    pub fn new(commands: Arc<dyn NodeCommands>) -> Self {
        let (watches, mut queue) = mpsc::unbounded_channel::<(NodeId, Uuid, bool)>();
        tokio::spawn(async move {
            while let Some((node, server_id, watch)) = queue.recv().await {
                send_watch(commands.as_ref(), node, server_id, watch).await;
            }
        });
        Self {
            inner: Arc::new(HubInner {
                watched: Mutex::new(HashMap::new()),
                watches,
            }),
        }
    }

    /// Inscrit un spectateur ; le premier déclenche le `server.watch` du nœud.
    pub fn join(&self, server_id: Uuid, node: NodeId) -> Viewer {
        let mut watched = self.watched();
        let entry = watched.entry(server_id).or_insert_with(|| Watched {
            node,
            viewers: 0,
            history: VecDeque::new(),
            events: broadcast::channel(VIEWER_QUEUE).0,
        });
        entry.viewers += 1;
        if entry.viewers == 1 {
            let _ = self.inner.watches.send((node, server_id, true));
        }
        Viewer {
            history: entry.history.iter().cloned().collect(),
            events: entry.events.subscribe(),
            hub: self.clone(),
            server_id,
        }
    }

    fn leave(&self, server_id: Uuid) {
        let mut watched = self.watched();
        let Some(entry) = watched.get_mut(&server_id) else {
            return;
        };
        entry.viewers = entry.viewers.saturating_sub(1);
        if entry.viewers == 0 {
            let node = entry.node;
            watched.remove(&server_id);
            let _ = self.inner.watches.send((node, server_id, false));
        }
    }

    /// Serveurs de ce nœud regardés en ce moment, à réabonner après une reconnexion.
    pub fn watched_on(&self, node: NodeId) -> Vec<Uuid> {
        self.watched()
            .iter()
            .filter(|(_, w)| w.node == node)
            .map(|(id, _)| *id)
            .collect()
    }

    /// Lignes reçues d'un nœud : ignorées si le serveur n'est pas regardé ou pas à lui.
    pub fn publish_lines(&self, node: NodeId, output: ConsoleOutput) {
        let mut watched = self.watched();
        let Some(entry) = self.owned(&mut watched, node, output.server_id) else {
            return;
        };
        // Après une reconnexion, le nœud renvoie son historique : les lignes déjà connues sont
        // écartées.
        let lines: Vec<ConsoleLine> = output
            .lines
            .into_iter()
            .filter(|line| !entry.knows(line))
            .collect();
        if lines.is_empty() && output.dropped == 0 {
            return;
        }
        for line in &lines {
            if entry.history.len() == CONSOLE_HISTORY_LINES {
                entry.history.pop_front();
            }
            entry.history.push_back(line.clone());
        }
        let _ = entry.events.send(ConsoleEvent::Lines {
            lines,
            dropped: output.dropped,
        });
    }

    pub fn publish_stats(&self, node: NodeId, stats: ServerStats) {
        let mut watched = self.watched();
        if let Some(entry) = self.owned(&mut watched, node, stats.server_id) {
            let _ = entry.events.send(ConsoleEvent::Stats {
                cpu_millis: stats.cpu_millis,
                memory_bytes: stats.memory_bytes,
                memory_limit_bytes: stats.memory_limit_bytes,
                network_rx_bytes: stats.network_rx_bytes,
                network_tx_bytes: stats.network_tx_bytes,
            });
        }
    }

    pub fn publish_state(&self, node: NodeId, report: &ServerStateReport) {
        let Some(state) = ServerState::from_reported(report.state) else {
            return;
        };
        let mut watched = self.watched();
        if let Some(entry) = self.owned(&mut watched, node, report.server_id) {
            let _ = entry.events.send(ConsoleEvent::State {
                state: state.as_str(),
                reason: report.reason.clone(),
            });
        }
    }

    fn owned<'a>(
        &self,
        watched: &'a mut HashMap<Uuid, Watched>,
        node: NodeId,
        server_id: Uuid,
    ) -> Option<&'a mut Watched> {
        match watched.get_mut(&server_id) {
            Some(entry) if entry.node == node => Some(entry),
            Some(_) => {
                // Un daemon n'agit que sur ses serveurs.
                tracing::warn!(%node, %server_id, "console data from a node that does not own the server");
                None
            }
            None => None,
        }
    }

    fn watched(&self) -> MutexGuard<'_, HashMap<Uuid, Watched>> {
        self.inner
            .watched
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

async fn send_watch(commands: &dyn NodeCommands, node: NodeId, server_id: Uuid, watch: bool) {
    match commands
        .send(
            node,
            ApiMessage::ServerWatch(ServerWatch { server_id, watch }),
        )
        .await
    {
        Ok(CommandResult::Ok) => {}
        // Nœud hors ligne : il sera réabonné à sa reconnexion.
        Ok(result) => tracing::debug!(%node, %server_id, ?result, "server.watch refused"),
        Err(failure) => tracing::debug!(%node, %server_id, %failure, "server.watch not sent"),
    }
}

/// Ticket d'ouverture de la console, tracé dans l'audit (`server.console_opened`).
pub async fn console_ticket(
    state: &ServersState,
    context: &AuditContext,
    id: Uuid,
    admin_id: Uuid,
) -> Result<String, ServersError> {
    let server = get(state, id).await?.server;
    // Tant que les flux ne traversent pas les instances (ADR-0037, étape 2), la console d'un
    // nœud tenu par une autre instance est refusée explicitement.
    let node = NodeId::from(server.node_id);
    if !state.commands.holds(node)
        && crate::nodes::infra::NodeRepository::new(state.pool.clone())
            .held_by(node)
            .await?
            .is_some()
    {
        return Err(ServersError::Conflict(ErrorCode::ConsoleHeldElsewhere));
    }
    let ticket = super::tickets::issue_console(&state.pool, id, admin_id).await?;
    let tx = AuditedTx::begin(&state.pool, context).await?;
    tx.commit(AuditEvent::success(AuditAction::ServerConsoleOpened).target("server", id))
        .await?;
    Ok(ticket)
}

/// Envoie une ligne sur la console du serveur. La ligne (tronquée) est gardée dans l'audit.
pub async fn command(
    state: &ServersState,
    context: &AuditContext,
    id: Uuid,
    line: &str,
) -> Result<(), ServersError> {
    if !is_valid_command(line) {
        return Err(ServersError::Invalid(FieldError::new(
            "line",
            ErrorCode::InvalidCommand,
        )));
    }
    let server = get(state, id).await?.server;
    super::manage::ensure_not_suspended(&server)?;
    if server.state != ServerState::Running.as_str()
        && server.state != ServerState::Starting.as_str()
    {
        return Err(ServersError::Conflict(ErrorCode::ServerNotRunning));
    }
    let result = state
        .commands
        .send(
            NodeId::from(server.node_id),
            ApiMessage::ServerCommand(ServerCommand {
                server_id: id,
                line: line.to_owned(),
            }),
        )
        .await?;
    if let CommandResult::Error {
        code: CommandErrorCode::NotRunning,
        ..
    } = result
    {
        return Err(ServersError::Conflict(ErrorCode::ServerNotRunning));
    }
    accepted(result)?;
    let audited: String = line.chars().take(AUDITED_COMMAND_CHARS).collect();
    // Audit ouvert une fois la ligne acceptée : aucune connexion tenue pendant l'attente du nœud.
    let tx = AuditedTx::begin(&state.pool, context).await?;
    tx.commit(
        AuditEvent::success(AuditAction::ServerCommand)
            .target("server", id)
            .details(json!({ "line": audited })),
    )
    .await?;
    Ok(())
}
