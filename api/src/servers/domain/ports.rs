//! Ports entre le domaine `servers` et le lien daemon (`crate::ws`), qui les implémente.

use std::fmt::Debug;

use futures_util::future::BoxFuture;
use kuro_protocol::daemon::server::{CommandResult, ServerStateReport};
use kuro_protocol::daemon::{BackupDone, ConsoleOutput, ContentDone, ServerDisk, ServerStats};
use kuro_protocol::{ApiMessage, DaemonMessage};
use uuid::Uuid;

use crate::nodes::domain::NodeId;

/// Pourquoi une commande n'a pas obtenu de réponse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CommandFailure {
    #[error("node is not connected")]
    Offline,
    #[error("node did not answer in time")]
    Timeout,
    #[error("link closed before the answer")]
    Closed,
    #[error("node answered with an unexpected message")]
    Unexpected,
}

/// Envoie une requête à un nœud connecté et attend sa réponse (`reply_to`).
pub trait NodeCommands: Send + Sync + Debug {
    /// La session du nœud est ouverte dans ce processus : ses flux (console, stats) arrivent ici
    /// (ADR-0037). Une commande, elle, atteint le nœud où qu'il soit tenu.
    fn holds(&self, node: NodeId) -> bool;

    /// Réponse quelconque : `command.result`, ou une réponse qui porte des données
    /// (`file.listing`…).
    fn request(
        &self,
        node: NodeId,
        message: ApiMessage,
    ) -> BoxFuture<'_, Result<DaemonMessage, CommandFailure>>;

    /// Commande dont la réponse est un `command.result`.
    fn send(
        &self,
        node: NodeId,
        message: ApiMessage,
    ) -> BoxFuture<'_, Result<CommandResult, CommandFailure>> {
        let reply = self.request(node, message);
        Box::pin(async move {
            match reply.await? {
                DaemonMessage::CommandResult(result) => Ok(result),
                _ => Err(CommandFailure::Unexpected),
            }
        })
    }
}

/// Ce que les sessions de lien demandent au domaine `servers`.
pub trait ServerEvents: Send + Sync + Debug {
    /// Serveurs assignés au nœud, envoyés dans `server.sync` après le `hello`.
    fn assigned(&self, node: NodeId) -> BoxFuture<'_, Result<Vec<Uuid>, sqlx::Error>>;

    /// État rapporté par le nœud. Ignoré (et journalisé) pour un serveur qui ne lui est pas
    /// assigné : un daemon n'agit que sur ses serveurs.
    fn record_state(&self, node: NodeId, report: ServerStateReport) -> BoxFuture<'_, ()>;

    /// Serveurs du nœud regardés en ce moment : `server.watch` à renvoyer après une reconnexion.
    fn watched(&self, node: NodeId) -> Vec<Uuid>;

    /// Console d'un serveur regardé, relayée aux spectateurs. Ignorée pour un serveur qui n'est
    /// pas au nœud.
    fn console(&self, node: NodeId, output: ConsoleOutput);

    /// Stats d'un serveur regardé, mêmes règles que [`Self::console`].
    fn stats(&self, node: NodeId, stats: ServerStats);

    /// Espace disque mesuré par le nœud (ADR-0019). Ignoré pour un serveur qui n'est pas au nœud.
    fn disk_usage(&self, node: NodeId, disk: ServerDisk);

    /// Fin d'une installation de contenu (ADR-0020). Ignorée pour un serveur qui n'est pas au
    /// nœud.
    fn content_done(&self, node: NodeId, done: ContentDone);

    /// Fin d'une sauvegarde (ADR-0018). Ignorée pour un serveur qui n'est pas au nœud.
    fn backup_done(&self, node: NodeId, done: BackupDone);

    /// Sauvegardes en cours des serveurs du nœud, (serveur, sauvegarde) : lues pendant la
    /// poignée de main, comme [`Self::assigned`].
    fn running_backups(
        &self,
        node: NodeId,
    ) -> BoxFuture<'_, Result<Vec<(Uuid, Uuid)>, sqlx::Error>>;

    /// Lien établi : rattrape ces sauvegardes, terminées ou perdues pendant la coupure.
    fn reconcile_backups(&self, node: NodeId, running: Vec<(Uuid, Uuid)>);
}
