//! Requêtes SFTP du daemon vers l'API (ADR-0047). Ce sont les seules que le daemon émet : l'API
//! y répond en `reply_to` (`sftp.grant`, `sftp.leases`, `sftp.audit.ack`, ou `error`).
//!
//! Le daemon ne les envoie que si `hello.ack` porte `sftp: true`.

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Authentification d'une connexion SFTP. Réponse `sftp.grant`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SftpAuth {
    /// Identifiant de la session SSH, choisi par le daemon (repris dans l'audit).
    pub session_id: Uuid,
    /// `<identifiant du serveur>.<identifiant du compte>`.
    pub username: String,
    pub credential: SftpCredential,
    /// Adresse du client SSH (pair TCP).
    pub client_ip: String,
    /// Version annoncée par le client (`SSH-2.0-…`), tronquée.
    pub client_version: String,
}

/// Preuve présentée par le client. Le mot de passe n'apparaît jamais dans `Debug`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum SftpCredential {
    /// Clé dont le client a prouvé la possession (signature vérifiée par le daemon).
    Publickey {
        /// `ssh-ed25519`, `ecdsa-sha2-nistp256`…
        algorithm: String,
        /// Clé publique au format du protocole SSH, en base64.
        key: String,
    },
    /// Mot de passe SFTP généré par le panel (tranche 3).
    Password { password: String },
}

impl fmt::Debug for SftpCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Publickey { algorithm, key } => f
                .debug_struct("Publickey")
                .field("algorithm", algorithm)
                .field("key", key)
                .finish(),
            Self::Password { .. } => f
                .debug_struct("Password")
                .field("password", &"<redacted>")
                .finish(),
        }
    }
}

/// Compte au nom duquel la session agit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum SftpPrincipal {
    Admin(Uuid),
}

/// Identifiant accordé par l'API pour une session : clé enregistrée ou mot de passe SFTP. Le
/// renouvellement du bail vérifie qu'il existe toujours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum SftpCredentialRef {
    Key(Uuid),
    Password(Uuid),
}

/// Renouvellement des baux des sessions ouvertes. Réponse `sftp.leases`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SftpLease {
    pub sessions: Vec<SftpLeaseRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SftpLeaseRequest {
    pub session_id: Uuid,
    pub server_id: Uuid,
    pub principal: SftpPrincipal,
    pub credential: SftpCredentialRef,
}

/// Événements d'audit des sessions, remontés par lots dans l'ordre. Réponse `sftp.audit.ack`.
/// L'API les dédoublonne par `event_id` : un lot renvoyé après une coupure n'est pas compté deux
/// fois.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SftpAudit {
    pub events: Vec<SftpEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SftpEvent {
    pub event_id: Uuid,
    /// Moment de l'événement sur le nœud (ms depuis l'époque Unix) ; l'API date l'enregistrement.
    pub occurred_at_ms: i64,
    pub session_id: Uuid,
    pub server_id: Uuid,
    pub principal: SftpPrincipal,
    pub client_ip: String,
    pub client_version: String,
    pub event: SftpEventKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", content = "details")]
pub enum SftpEventKind {
    #[serde(rename = "session.closed")]
    SessionClosed(SessionSummary),
    /// Opération refusée faute de permission ; une fois par session et par permission.
    #[serde(rename = "access.denied")]
    AccessDenied { permission: String },
    /// Fichier écrit, à sa fermeture (pas à chaque paquet). Chemins `/<service>/<volume>/…`,
    /// comme l'explorateur du panel.
    #[serde(rename = "file.written")]
    FileWritten { path: String, size: u64 },
    #[serde(rename = "directory.created")]
    DirectoryCreated { path: String },
    #[serde(rename = "file.renamed")]
    FileRenamed { from: String, to: String },
    /// Fichier, lien ou dossier vide supprimé.
    #[serde(rename = "file.deleted")]
    FileDeleted { path: String },
    #[serde(rename = "file.mode_changed")]
    ModeChanged { path: String, mode: u32 },
}

impl SftpEventKind {
    /// Événement d'une écriture (fichier, dossier, renommage, suppression, droits).
    pub fn is_write(&self) -> bool {
        !matches!(self, Self::SessionClosed(_) | Self::AccessDenied { .. })
    }
}

/// Résumé d'une session fermée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub duration_secs: u64,
    pub reason: CloseReason,
    pub bytes_read: u64,
    pub files_read: u64,
    pub bytes_written: u64,
    pub files_written: u64,
    /// Opérations refusées (chemin, lien, permission).
    pub denied: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseReason {
    /// Le client s'est déconnecté.
    Client,
    Idle,
    /// Bail refusé ou non renouvelé (droit retiré, lien avec l'API coupé).
    LeaseDenied,
    ServerSuspended,
    ServerDeleted,
    /// Arrêt du service SFTP ou du daemon.
    Shutdown,
    #[serde(other)]
    Other,
}
