//! Installation des serveurs (ADR-0017) : téléchargements vérifiés et extraction confinée. Les
//! étapes en conteneur (`steamcmd`, `script`) sont pilotées par le gestionnaire de serveurs.

pub mod download;
pub mod extract;

use crate::fs::volumes::VolumeError;

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("download failed: {0}")]
    Download(String),
    #[error("checksum mismatch: expected {expected}, got {actual}")]
    Checksum { expected: String, actual: String },
    #[error("{0}")]
    Failed(String),
    /// Conteneur d'installation sorti en erreur (code de sortie).
    #[error("install step exited with code {0}")]
    Exited(i64),
    #[error("volume: {0}")]
    Volume(#[from] VolumeError),
}
