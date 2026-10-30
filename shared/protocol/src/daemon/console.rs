//! Console et stats d'un serveur regardé (ADR-0014), envoyées tant que dure le `server.watch`.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Lot de lignes de la console du service principal (type `console.line`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsoleOutput {
    pub server_id: Uuid,
    pub lines: Vec<ConsoleLine>,
    /// Lignes perdues depuis le lot précédent, faute de débit sur le lien.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub dropped: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsoleLine {
    /// Horodatage Docker, en millisecondes Unix.
    pub at_ms: i64,
    /// Texte brut (séquences ANSI comprises), tronqué à
    /// [`crate::limits::MAX_CONSOLE_LINE_BYTES`].
    pub text: String,
}

/// Consommation d'un serveur, somme de ses conteneurs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerStats {
    pub server_id: Uuid,
    /// 1000 = un cœur plein.
    pub cpu_millis: u64,
    /// Mémoire utilisée, hors cache de pages.
    pub memory_bytes: u64,
    pub memory_limit_bytes: u64,
    /// Compteurs cumulés depuis le démarrage des conteneurs.
    pub network_rx_bytes: u64,
    pub network_tx_bytes: u64,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}
