//! Réglages du projet qui concernent tout le nœud : donnés dans `hello.ack`, puis par
//! `node.settings` à chaque changement (réponse `command.result`).

use serde::{Deserialize, Serialize};

/// Rétention de l'historique des fichiers (ADR-0049).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryPolicy {
    /// Durée de conservation d'une version ; 0 : historique coupé (rien de gardé, tout effacé).
    pub days: u32,
    /// Place prise au plus par l'historique d'un serveur.
    pub max_mb: u32,
}

impl Default for HistoryPolicy {
    fn default() -> Self {
        Self {
            days: 30,
            max_mb: 100,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeSettings {
    pub file_history: HistoryPolicy,
}
