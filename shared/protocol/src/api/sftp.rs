//! Réponses de l'API aux requêtes SFTP du daemon (ADR-0047), en `reply_to`.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::daemon::sftp::{SftpCredentialRef, SftpPrincipal};

/// Réponse à `sftp.auth`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum SftpGrant {
    Granted {
        /// Serveur désigné par le nom d'utilisateur, assigné à ce nœud.
        server_id: Uuid,
        principal: SftpPrincipal,
        credential: SftpCredentialRef,
        /// `servers.files.write` : écriture autorisée.
        write: bool,
        /// Durée du bail : sans renouvellement accordé, la session est fermée.
        lease_secs: u32,
    },
    Denied {
        reason: SftpDenial,
    },
}

/// Raison d'un refus, pour les logs du daemon. Le client SSH ne reçoit qu'un échec ordinaire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SftpDenial {
    /// Nom d'utilisateur mal formé.
    InvalidUsername,
    UnknownAccount,
    UnknownServer,
    UnknownKey,
    BadPassword,
    ServerSuspended,
    PermissionDenied,
    /// SFTP coupé dans les paramètres du projet.
    Disabled,
    #[serde(other)]
    Other,
}

/// Réponse à `sftp.lease`, dans l'ordre des sessions demandées.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SftpLeases {
    pub results: Vec<SftpLeaseResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SftpLeaseResult {
    pub session_id: Uuid,
    pub granted: bool,
    pub write: bool,
}

/// Réponse à `sftp.audit` : le lot est enregistré, le daemon peut l'oublier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SftpAuditAck {
    /// Événements nouveaux ; les autres étaient déjà enregistrés.
    pub recorded: u32,
}
