//! Commandes vers un nœud tenu par une autre instance de l'API (ADR-0037).
//!
//! Un nœud relié à ce processus reçoit ses commandes par le registre local, comme avant. Sinon
//! la commande est déposée en base (`node_commands`, message entier) et l'instance qui tient le
//! nœud est réveillée par le bus ; elle l'envoie, écrit la réponse dans la même ligne et réveille
//! l'émetteur. Un balayage de secours toutes les deux secondes rattrape une notification perdue.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures_util::future::BoxFuture;
use kuro_protocol::{ApiMessage, DaemonMessage};
use sqlx::PgPool;
use tokio::sync::oneshot;
use uuid::Uuid;

use super::registry::{COMMAND_TIMEOUT, LinkRegistry};
use crate::cluster::Bus;
use crate::nodes::domain::{DisconnectReason, InstanceId, NodeId, NodeLinks};
use crate::nodes::infra::NodeRepository;
use crate::servers::domain::{CommandFailure, NodeCommands};

/// Canal du bus : une commande attend l'instance dont l'identifiant est le payload.
pub const COMMANDS_CHANNEL: &str = "kuro_node_commands";
/// Canal du bus : la commande dont l'identifiant est le payload a sa réponse.
pub const REPLIES_CHANNEL: &str = "kuro_node_replies";
/// Secours : commandes en attente relues, réponses revérifiées.
pub const FALLBACK_POLL: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub struct CommandRelay {
    local: Arc<LinkRegistry>,
    pool: PgPool,
    nodes: NodeRepository,
    instance: InstanceId,
    /// Commandes déposées par ce processus, en attente de leur réponse.
    waiting: Mutex<HashMap<Uuid, oneshot::Sender<()>>>,
}

impl CommandRelay {
    pub fn new(local: Arc<LinkRegistry>, pool: PgPool, instance: InstanceId) -> Arc<Self> {
        Arc::new(Self {
            local,
            nodes: NodeRepository::new(pool.clone()),
            pool,
            instance,
            waiting: Mutex::new(HashMap::new()),
        })
    }

    /// Abonne le relais au bus : commandes déposées pour cette instance, réponses attendues ici.
    pub fn subscribe(self: &Arc<Self>, bus: Bus) -> Bus {
        let on_command = {
            let relay = Arc::clone(self);
            move |payload: &str| {
                if payload == relay.instance.to_string() {
                    let relay = Arc::clone(&relay);
                    tokio::spawn(async move { relay.execute_pending().await });
                }
            }
        };
        let on_reply = {
            let relay = Arc::clone(self);
            move |payload: &str| {
                if let Ok(id) = payload.parse::<Uuid>() {
                    relay.wake(id);
                }
            }
        };
        let on_tick = {
            let relay = Arc::clone(self);
            move || {
                let relay = Arc::clone(&relay);
                tokio::spawn(async move { relay.execute_pending().await });
            }
        };
        bus.subscribe(COMMANDS_CHANNEL, Arc::new(on_command))
            .subscribe(REPLIES_CHANNEL, Arc::new(on_reply))
            .every(FALLBACK_POLL, Arc::new(on_tick))
    }

    fn wake(&self, id: Uuid) {
        if let Some(sender) = self.waiting().remove(&id) {
            let _ = sender.send(());
        }
    }

    fn waiting(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, oneshot::Sender<()>>> {
        self.waiting.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Commandes déposées pour cette instance : chacune prise une seule fois, envoyée au nœud
    /// par le registre local, puis répondue.
    pub async fn execute_pending(self: &Arc<Self>) {
        let claimed = sqlx::query!(
            r#"UPDATE node_commands SET claimed_at = now()
               WHERE id IN (SELECT id FROM node_commands
                            WHERE instance_id = $1 AND replied_at IS NULL AND claimed_at IS NULL
                            ORDER BY created_at
                            FOR UPDATE SKIP LOCKED)
               RETURNING id, node_id, message"#,
            self.instance.as_uuid()
        )
        .fetch_all(&self.pool)
        .await;
        let claimed = match claimed {
            Ok(claimed) => claimed,
            Err(error) => {
                tracing::warn!(%error, "failed to claim forwarded commands");
                return;
            }
        };
        for row in claimed {
            let relay = Arc::clone(self);
            tokio::spawn(async move {
                let outcome = match serde_json::from_value::<ApiMessage>(row.message) {
                    Ok(message) => {
                        relay
                            .local
                            .request(NodeId::from(row.node_id), message)
                            .await
                    }
                    // Message d'une version plus récente de l'API : injouable ici.
                    Err(_) => Err(CommandFailure::Unexpected),
                };
                relay.answer(row.id, outcome).await;
            });
        }
    }

    async fn answer(&self, id: Uuid, outcome: Result<DaemonMessage, CommandFailure>) {
        let (reply, failure) = match outcome {
            Ok(message) => (serde_json::to_value(&message).ok(), None),
            Err(failure) => (None, Some(failure_name(failure))),
        };
        let written = sqlx::query!(
            "UPDATE node_commands SET reply = $2, failure = $3, replied_at = now() WHERE id = $1",
            id,
            reply,
            failure
        )
        .execute(&self.pool)
        .await;
        if let Err(error) = written {
            tracing::warn!(%error, command = %id, "failed to answer a forwarded command");
            return;
        }
        if let Err(error) = Bus::notify(&self.pool, REPLIES_CHANNEL, &id.to_string()).await {
            tracing::debug!(%error, "reply notification not sent, the caller will poll");
        }
    }

    /// Dépose la commande pour l'instance qui tient le nœud et attend sa réponse.
    async fn forward(
        &self,
        node: NodeId,
        message: ApiMessage,
    ) -> Result<DaemonMessage, CommandFailure> {
        let holder = self.nodes.held_by(node).await.map_err(|error| {
            tracing::warn!(%error, "failed to find the node's instance");
            CommandFailure::Offline
        })?;
        let target = match holder {
            // Tenu par nous en base mais plus en registre : la session se ferme.
            Some(target) if target != self.instance => target,
            _ => return Err(CommandFailure::Offline),
        };
        let id = Uuid::now_v7();
        let message = serde_json::to_value(&message).map_err(|_| CommandFailure::Unexpected)?;
        sqlx::query!(
            "INSERT INTO node_commands (id, node_id, instance_id, message) VALUES ($1, $2, $3, $4)",
            id,
            node.as_uuid(),
            target.as_uuid(),
            message
        )
        .execute(&self.pool)
        .await
        .map_err(|error| {
            tracing::warn!(%error, "failed to forward a command");
            CommandFailure::Offline
        })?;
        let mut woken = self.wait_for(id);
        if let Err(error) = Bus::notify(&self.pool, COMMANDS_CHANNEL, &target.to_string()).await {
            tracing::debug!(%error, "command notification not sent, the holder will poll");
        }
        let deadline = tokio::time::Instant::now() + COMMAND_TIMEOUT;
        loop {
            tokio::select! {
                _ = &mut woken => {}
                () = tokio::time::sleep(FALLBACK_POLL) => {}
                () = tokio::time::sleep_until(deadline) => {
                    self.waiting().remove(&id);
                    return Err(CommandFailure::Timeout);
                }
            }
            if let Some(outcome) = self.outcome(id).await {
                self.waiting().remove(&id);
                return outcome;
            }
            woken = self.wait_for(id);
        }
    }

    fn wait_for(&self, id: Uuid) -> oneshot::Receiver<()> {
        let (sender, receiver) = oneshot::channel();
        self.waiting().insert(id, sender);
        receiver
    }

    /// Réponse écrite par l'instance qui tient le nœud, si elle l'est déjà.
    async fn outcome(&self, id: Uuid) -> Option<Result<DaemonMessage, CommandFailure>> {
        let row = sqlx::query!(
            "SELECT reply, failure FROM node_commands WHERE id = $1 AND replied_at IS NOT NULL",
            id
        )
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten()?;
        Some(match (row.reply, row.failure.as_deref()) {
            (Some(reply), _) => {
                serde_json::from_value(reply).map_err(|_| CommandFailure::Unexpected)
            }
            (None, Some("offline")) => Err(CommandFailure::Offline),
            (None, Some("timeout")) => Err(CommandFailure::Timeout),
            (None, _) => Err(CommandFailure::Closed),
        })
    }
}

fn failure_name(failure: CommandFailure) -> &'static str {
    match failure {
        CommandFailure::Offline => "offline",
        CommandFailure::Timeout => "timeout",
        CommandFailure::Closed | CommandFailure::Unexpected => "closed",
    }
}

impl NodeCommands for CommandRelay {
    fn holds(&self, node: NodeId) -> bool {
        NodeLinks::holds(self.local.as_ref(), node)
    }

    fn request(
        &self,
        node: NodeId,
        message: ApiMessage,
    ) -> BoxFuture<'_, Result<DaemonMessage, CommandFailure>> {
        Box::pin(async move {
            if NodeLinks::holds(self.local.as_ref(), node) {
                self.local.request(node, message).await
            } else {
                self.forward(node, message).await
            }
        })
    }
}

impl NodeLinks for CommandRelay {
    fn disconnect(&self, node: NodeId, reason: DisconnectReason) {
        self.local.disconnect(node, reason);
    }

    fn holds(&self, node: NodeId) -> bool {
        NodeLinks::holds(self.local.as_ref(), node)
    }
}
