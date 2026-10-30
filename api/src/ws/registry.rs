//! Connexions daemon actives : une seule par nœud, la plus récente gagne.
//!
//! Partagé entre le listener (qui enregistre les sessions) et l'état REST, il implémente les
//! ports [`NodeLinks`] (couper une session) et [`NodeCommands`] (envoyer une commande et
//! attendre sa réponse).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use std::time::Duration;

use futures_util::future::BoxFuture;
use kuro_protocol::{ApiMessage, DaemonMessage};
use tokio::sync::{mpsc, oneshot};

use crate::nodes::domain::{DisconnectReason, NodeId, NodeLinks};
use crate::servers::domain::{CommandFailure, NodeCommands};

/// Délai de réponse d'un daemon à une commande (acceptation, pas exécution).
pub(super) const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
/// Commandes en attente d'envoi par session.
const COMMAND_QUEUE: usize = 32;

/// Commande à envoyer par une session, avec de quoi lui transmettre la réponse.
#[derive(Debug)]
pub(super) struct Outgoing {
    pub(super) message: ApiMessage,
    pub(super) reply: oneshot::Sender<DaemonMessage>,
}

/// Identifiant d'une connexion, pour distinguer l'ancienne de la nouvelle d'un même nœud.
pub(super) type ConnectionId = u64;

/// Pourquoi une session doit se terminer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Eviction {
    /// Une connexion plus récente du même nœud la remplace.
    Replaced,
    /// Décision d'administration : le nœud est informé puis déconnecté.
    Disconnected(DisconnectReason),
}

#[derive(Debug, Default)]
pub struct LinkRegistry {
    connections: Mutex<HashMap<NodeId, Entry>>,
    next_id: AtomicU64,
}

#[derive(Debug)]
struct Entry {
    connection: ConnectionId,
    /// Consommé par la première éviction.
    evict: Option<oneshot::Sender<Eviction>>,
    commands: mpsc::Sender<Outgoing>,
}

/// Session enregistrée : son identifiant, son éviction et ses commandes à envoyer.
pub(super) struct Registration {
    pub(super) connection: ConnectionId,
    pub(super) evicted: oneshot::Receiver<Eviction>,
    pub(super) commands: mpsc::Receiver<Outgoing>,
}

impl LinkRegistry {
    /// Enregistre une connexion et évince celle qui existait pour ce nœud.
    /// `evicted` se résout quand cette connexion est à son tour évincée.
    pub(super) fn register(&self, node: NodeId) -> Registration {
        let connection = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (evict, evicted) = oneshot::channel();
        let (commands_tx, commands) = mpsc::channel(COMMAND_QUEUE);
        let previous = self.lock().insert(
            node,
            Entry {
                connection,
                evict: Some(evict),
                commands: commands_tx,
            },
        );
        if let Some(sender) = previous.and_then(|entry| entry.evict) {
            // L'ancienne session a pu se terminer entre-temps : l'échec est sans conséquence.
            let _ = sender.send(Eviction::Replaced);
        }
        Registration {
            connection,
            evicted,
            commands,
        }
    }

    /// Retire la connexion si elle est toujours la connexion courante du nœud.
    /// Retourne `false` si elle a été remplacée : le nœud reste alors `online`.
    pub(super) fn unregister(&self, node: NodeId, connection: ConnectionId) -> bool {
        let mut connections = self.lock();
        match connections.get(&node) {
            Some(entry) if entry.connection == connection => {
                connections.remove(&node);
                true
            }
            _ => false,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<NodeId, Entry>> {
        // Aucune opération sous le verrou ne peut laisser la map incohérente.
        self.connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl NodeLinks for LinkRegistry {
    fn disconnect(&self, node: NodeId, reason: DisconnectReason) {
        let sender = self
            .lock()
            .get_mut(&node)
            .and_then(|entry| entry.evict.take());
        if let Some(sender) = sender {
            let _ = sender.send(Eviction::Disconnected(reason));
        }
    }

    fn holds(&self, node: NodeId) -> bool {
        self.lock().contains_key(&node)
    }
}

impl NodeCommands for LinkRegistry {
    fn holds(&self, node: NodeId) -> bool {
        NodeLinks::holds(self, node)
    }

    fn request(
        &self,
        node: NodeId,
        message: ApiMessage,
    ) -> BoxFuture<'_, Result<DaemonMessage, CommandFailure>> {
        let sender = self.lock().get(&node).map(|entry| entry.commands.clone());
        Box::pin(async move {
            let sender = sender.ok_or(CommandFailure::Offline)?;
            let (reply, response) = oneshot::channel();
            sender
                .send(Outgoing { message, reply })
                .await
                .map_err(|_| CommandFailure::Offline)?;
            match tokio::time::timeout(COMMAND_TIMEOUT, response).await {
                Ok(Ok(result)) => Ok(result),
                Ok(Err(_)) => Err(CommandFailure::Closed),
                Err(_) => Err(CommandFailure::Timeout),
            }
        })
    }
}
