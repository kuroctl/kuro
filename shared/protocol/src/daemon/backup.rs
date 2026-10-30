//! Réponses et événements du daemon sur les sauvegardes (ADR-0018).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Fin d'un `backup.create`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupDone {
    pub server_id: Uuid,
    pub backup_id: Uuid,
    #[serde(flatten)]
    pub outcome: BackupOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BackupOutcome {
    Ok {
        size: u64,
        /// SHA-256 hexadécimal de l'archive.
        sha256: String,
    },
    Failed {
        /// Cause, sans secret.
        reason: String,
    },
}

/// Réponse à `backup.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupListing {
    pub backups: Vec<BackupInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupInfo {
    pub backup_id: Uuid,
    #[serde(flatten)]
    pub state: BackupState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum BackupState {
    /// Archivage en cours.
    Creating,
    Ready {
        size: u64,
        sha256: String,
    },
}

/// Réponse à `backup.read`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupChunk {
    /// Octets lus, en base64 standard ; au plus [`crate::limits::BACKUP_CHUNK_BYTES`].
    pub data: String,
    /// Fin de l'archive atteinte.
    pub eof: bool,
}
