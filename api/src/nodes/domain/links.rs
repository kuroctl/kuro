//! Port vers les sessions de lien ouvertes avec les nœuds, implémenté par le transport (`ws`).
//! Le domaine décide de couper une session ; il ne sait pas comment.

use super::NodeId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisconnectReason {
    Revoked,
    Deleted,
}

impl DisconnectReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Revoked => "node revoked",
            Self::Deleted => "node deleted",
        }
    }
}

pub trait NodeLinks: Send + Sync + std::fmt::Debug {
    /// Coupe la session du nœud dans ce processus, s'il en a une.
    fn disconnect(&self, node: NodeId, reason: DisconnectReason);

    /// La session du nœud est ouverte dans ce processus (ADR-0037) : ses flux (console, stats)
    /// arrivent ici.
    fn holds(&self, node: NodeId) -> bool;
}

/// Aucune session dans ce processus (CLI, autre instance) : la session vivante, où qu'elle soit,
/// constatera la révocation à son prochain heartbeat.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoLiveLinks;

impl NodeLinks for NoLiveLinks {
    fn disconnect(&self, _node: NodeId, _reason: DisconnectReason) {}

    fn holds(&self, _node: NodeId) -> bool {
        false
    }
}
