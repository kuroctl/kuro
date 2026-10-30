//! Sauvegardes d'un serveur (ADR-0018). Les archives restent sur le nœud ; l'API les crée, les
//! restaure, les supprime et les lit par morceaux (téléchargement).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Archive les volumes du serveur ; acceptée tout de suite, terminée par `backup.done`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupCreate {
    pub server_id: Uuid,
    pub backup_id: Uuid,
    /// Motifs exclus, en chemins de volumes (`server/data/logs/**`) : `*` couvre un segment,
    /// `**` un nombre quelconque de segments.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ignore: Vec<String>,
    /// Commandes console avant l'archivage, serveur en marche seulement.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub before: Vec<BackupHook>,
    /// Commandes console après l'archivage, même en cas d'échec.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub after: Vec<BackupHook>,
}

/// Commande envoyée au service principal, avec éventuellement une ligne attendue en retour.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupHook {
    pub console: String,
    /// Expression régulière attendue sur la console avant de continuer (délai borné par le
    /// daemon).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait: Option<String>,
}

/// Restaure une sauvegarde ; le serveur est arrêté d'abord. Terminée par des `server.state`
/// (`restoring`, puis `offline` ou `failed`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupRestore {
    pub server_id: Uuid,
    pub backup_id: Uuid,
    pub mode: RestoreMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestoreMode {
    /// Les fichiers de l'archive remplacent les leurs ; les autres restent.
    Overwrite,
    /// Volumes vidés, puis archive extraite.
    Clean,
}

/// Supprime une sauvegarde (archive et métadonnées) ; une sauvegarde absente n'est pas une erreur.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupDelete {
    pub server_id: Uuid,
    pub backup_id: Uuid,
}

/// Sauvegardes connues du nœud pour ce serveur ; réponse `backup.listing`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupList {
    pub server_id: Uuid,
}

/// Morceau d'une archive à partir de `offset` ; réponse `backup.chunk`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupRead {
    pub server_id: Uuid,
    pub backup_id: Uuid,
    pub offset: u64,
}
