use serde::{Deserialize, Serialize};

/// Refus d'un message, en général suivi de la fermeture du lien.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolError {
    pub code: ErrorCode,
    /// Détail destiné aux logs, sans secret.
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    UnsupportedProtocolVersion,
    /// Message illisible : JSON invalide, type inconnu, trop gros…
    InvalidMessage,
    /// Message valide mais hors séquence, par exemple un `heartbeat` avant `hello`.
    UnexpectedMessage,
    /// Nœud révoqué ou supprimé : le daemon cesse de se reconnecter (réenrôlement nécessaire).
    NodeRevoked,
    /// Code introduit par une version plus récente de l'API.
    #[serde(other)]
    Unknown,
}
