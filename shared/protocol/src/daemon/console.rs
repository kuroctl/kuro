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

/// Consommation d'un serveur sur une minute révolue (ADR-0043), envoyée pour tout serveur en
/// marche, regardé ou non ; l'API en garde l'historique. Les compteurs réseau sont ceux de la
/// minute (différence entre la première et la dernière mesure), pas des cumuls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerMetrics {
    pub server_id: Uuid,
    /// Début de la minute, en millisecondes Unix (multiple de 60 000).
    pub from_ms: i64,
    /// Fin de la minute (`from_ms` + 60 000).
    pub to_ms: i64,
    /// Mesures prises dans la minute (au plus six, une toutes les 10 s).
    pub samples: u32,
    pub cpu_millis_avg: u64,
    pub cpu_millis_max: u64,
    pub memory_bytes_avg: u64,
    pub memory_bytes_max: u64,
    pub memory_limit_bytes: u64,
    pub network_rx_bytes: u64,
    pub network_tx_bytes: u64,
}

#[cfg(test)]
mod metrics_tests {
    use super::*;

    #[test]
    fn server_metrics_round_trip_as_a_daemon_message() {
        let metrics = ServerMetrics {
            server_id: Uuid::nil(),
            from_ms: 1_800_000_000_000,
            to_ms: 1_800_000_060_000,
            samples: 6,
            cpu_millis_avg: 250,
            cpu_millis_max: 900,
            memory_bytes_avg: 1 << 30,
            memory_bytes_max: 2 << 30,
            memory_limit_bytes: 4 << 30,
            network_rx_bytes: 1234,
            network_tx_bytes: 5678,
        };
        let message = crate::DaemonMessage::ServerMetrics(metrics.clone());
        let json = serde_json::to_value(&message).unwrap();
        assert_eq!(json["type"], "server.metrics");
        assert_eq!(json["payload"]["samples"], 6);
        let back: crate::DaemonMessage = serde_json::from_value(json).unwrap();
        assert_eq!(back, crate::DaemonMessage::ServerMetrics(metrics));
    }
}
