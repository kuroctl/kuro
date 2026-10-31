//! Présence des nœuds : identification au lien, passage `online` / `offline`, heartbeats.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::instances::Instance;
use crate::nodes::domain::{CertFingerprint, Node, NodeHost, NodeId};
use crate::nodes::infra::NodeRepository;
use crate::notifications::domain::{Event, EventKind, NodeRef};

/// Période d'écriture des heartbeats reçus : une requête pour tous les nœuds, au lieu d'un
/// `UPDATE` par nœud toutes les 15 s. `last_heartbeat_at` est donc précis à cette période près.
pub const HEARTBEAT_FLUSH_EVERY: Duration = Duration::from_secs(30);

/// Situation d'un nœud dont la session est ouverte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkStanding {
    Active,
    Revoked,
    Deleted,
    /// Plus `online` pour une autre raison (balayé comme tenu par une instance morte) : la
    /// session se ferme, le daemon se reconnectera.
    Inactive,
}

#[derive(Debug, Clone)]
pub struct Presence {
    nodes: NodeRepository,
    /// Instance qui tient les sessions passées `online` par cette présence (ADR-0037).
    instance: Instance,
    /// Nœuds dont un heartbeat attend d'être écrit.
    seen: Arc<Mutex<HashSet<Uuid>>>,
}

impl Presence {
    pub fn new(nodes: NodeRepository, instance: Instance) -> Self {
        Self {
            nodes,
            instance,
            seen: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Nœud correspondant au certificat client présenté, s'il existe.
    pub async fn identify(
        &self,
        fingerprint: &CertFingerprint,
    ) -> Result<Option<Node>, sqlx::Error> {
        self.nodes.find_by_fingerprint(fingerprint).await
    }

    /// Certificat d'un nœud supprimé (ADR-0046) : refusé comme révoqué, le daemon s'arrête.
    pub async fn certificate_revoked(
        &self,
        fingerprint: &CertFingerprint,
    ) -> Result<bool, sqlx::Error> {
        self.nodes.certificate_revoked(fingerprint).await
    }

    /// `hello` accepté : le nœud est joignable, tenu par cette instance, et ses
    /// caractéristiques sont à jour. `false` si le nœud a été révoqué ou supprimé depuis son
    /// identification.
    pub async fn online(&self, id: NodeId, host: &NodeHost) -> Result<bool, sqlx::Error> {
        let was_online = self.nodes.status_of(id).await?.as_deref() == Some("online");
        let accepted = self.nodes.mark_online(id, host, self.instance.id).await?;
        if accepted && !was_online {
            self.notify(id, EventKind::NodeOnline).await?;
        }
        Ok(accepted)
    }

    /// Événement de nœud (ADR-0044), avec son nom.
    async fn notify(&self, id: NodeId, kind: EventKind) -> Result<(), sqlx::Error> {
        let mut conn = self.nodes.pool().acquire().await?;
        if let Some(node) = NodeRepository::get(&mut conn, id.as_uuid()).await? {
            let event = Event::now(kind).node(NodeRef {
                id: node.id,
                name: node.name,
            });
            crate::notifications::application::emit_detached(self.nodes.pool(), event);
        }
        Ok(())
    }

    /// Heartbeat reçu : compté pour la prochaine écriture groupée. Révèle aussi une révocation
    /// décidée ailleurs (CLI, autre instance de l'API) : la session vivante la constate au plus
    /// tard un intervalle de heartbeat après.
    pub async fn heartbeat(&self, id: NodeId) -> Result<LinkStanding, sqlx::Error> {
        Ok(match self.nodes.status_of(id).await?.as_deref() {
            Some("online") => {
                self.seen().insert(id.as_uuid());
                LinkStanding::Active
            }
            Some("revoked") => LinkStanding::Revoked,
            Some(_) => LinkStanding::Inactive,
            None => LinkStanding::Deleted,
        })
    }

    pub async fn offline(&self, id: NodeId) -> Result<(), sqlx::Error> {
        let was_online = self.nodes.status_of(id).await?.as_deref() == Some("online");
        self.nodes.mark_offline(id).await?;
        if was_online {
            self.notify(id, EventKind::NodeOffline).await?;
        }
        Ok(())
    }

    /// Écrit la date des heartbeats reçus depuis la dernière écriture ; retourne le nombre de
    /// nœuds touchés.
    pub async fn flush_heartbeats(&self) -> Result<u64, sqlx::Error> {
        let ids: Vec<Uuid> = std::mem::take(&mut *self.seen()).into_iter().collect();
        if ids.is_empty() {
            return Ok(0);
        }
        self.nodes.touch_heartbeats(&ids).await
    }

    /// Écrit les heartbeats toutes les [`HEARTBEAT_FLUSH_EVERY`], et une dernière fois à l'arrêt.
    pub async fn flush_forever(self, shutdown: CancellationToken) {
        let mut ticker = tokio::time::interval(HEARTBEAT_FLUSH_EVERY);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker.tick().await;
        loop {
            tokio::select! {
                () = shutdown.cancelled() => break,
                _ = ticker.tick() => {}
            }
            if let Err(error) = self.flush_heartbeats().await {
                tracing::warn!(%error, "failed to record node heartbeats");
            }
        }
        if let Err(error) = self.flush_heartbeats().await {
            tracing::warn!(%error, "failed to record node heartbeats");
        }
    }

    /// Au démarrage : les nœuds laissés `online` par une instance morte, ou par aucune, sont
    /// un reliquat d'un arrêt brutal. Ceux des instances vivantes ne sont pas touchés.
    pub async fn reset(&self) -> Result<u64, sqlx::Error> {
        Ok(self.instance.sweep().await?.nodes)
    }

    fn seen(&self) -> std::sync::MutexGuard<'_, HashSet<Uuid>> {
        self.seen.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
