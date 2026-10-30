//! Réponses et événements du daemon sur les serveurs (ADR-0013).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Réponse (`reply_to`) à une commande de l'API : acceptée ou refusée. Les opérations longues se
/// terminent ensuite par des `server.state`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CommandResult {
    Ok,
    Error {
        code: CommandErrorCode,
        /// Détail destiné aux logs, sans secret.
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandErrorCode {
    /// Spécification refusée par les règles du daemon (utilisateur root, chemin hors volume…).
    InvalidSpec,
    /// Serveur inconnu de ce nœud.
    UnknownServer,
    /// Une opération est déjà en cours sur ce serveur.
    Busy,
    /// Commande console refusée : le service principal ne tourne pas.
    NotRunning,
    /// Chemin de fichier refusé (hors volumes, `..`, lien symbolique…).
    InvalidPath,
    NotFound,
    AlreadyExists,
    /// Le fichier a changé depuis sa lecture (`if_sha256`).
    Conflict,
    /// Fichier binaire ou non UTF-8.
    NotText,
    TooLarge,
    /// Téléchargement impossible (adresse refusée, HTTP, taille).
    DownloadFailed,
    /// Le fichier téléchargé ne correspond pas au sha256 déclaré.
    ChecksumMismatch,
    /// Étape d'installation en échec (archive, SteamCMD, script).
    InstallFailed,
    DockerUnavailable,
    /// Quota disque du serveur atteint (ADR-0019).
    DiskQuotaExceeded,
    /// Serveur suspendu (ADR-0019).
    Suspended,
    Internal,
    /// Code introduit par une version plus récente du daemon.
    #[serde(other)]
    Unknown,
}

/// État d'un serveur constaté par le daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerStateReport {
    pub server_id: Uuid,
    pub state: ServerState,
    /// Cause d'un échec, sans secret.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerState {
    /// Images en téléchargement, conteneurs en création.
    Deploying,
    /// Étapes d'installation en cours (ADR-0017).
    Installing,
    /// Sauvegarde en cours de restauration (ADR-0018).
    Restoring,
    Offline,
    Starting,
    Running,
    Stopping,
    Failed,
    /// Conteneurs, réseau et données supprimés.
    Deleted,
}

/// Espace disque occupé par un serveur : volumes et sauvegardes (ADR-0019).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerDisk {
    pub server_id: Uuid,
    pub used_bytes: u64,
}
