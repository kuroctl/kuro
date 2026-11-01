//! Réponses du daemon aux lectures de fichiers (ADR-0015), en `reply_to` de la requête.

use serde::{Deserialize, Serialize};

/// Réponse à `file.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileListing {
    pub entries: Vec<FileEntry>,
    /// Le dossier compte plus d'entrées que le nœud n'en envoie : seules les premières sont là.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub kind: FileKind,
    pub size: u64,
    pub modified_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    File,
    Directory,
    /// Listé, supprimable, jamais suivi.
    Symlink,
    /// Socket, périphérique… : listé seulement.
    #[serde(other)]
    Other,
}

/// Réponse à `file.read`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileContent {
    pub content: String,
    pub size: u64,
    pub modified_ms: i64,
    /// SHA-256 hexadécimal du contenu, à renvoyer dans `file.write` (`if_sha256`).
    pub sha256: String,
}

/// Réponse à `file.history` (ADR-0049).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileVersions {
    pub versions: Vec<FileVersion>,
}

/// Version gardée d'un fichier : son contenu **avant** l'écriture décrite par `reason`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileVersion {
    pub id: uuid::Uuid,
    /// Moment où elle a été remplacée (ms depuis l'époque Unix).
    pub at_ms: i64,
    pub size: u64,
    pub sha256: String,
    pub reason: VersionReason,
    pub channel: VersionChannel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<crate::api::FileAuthor>,
}

/// Ce qui l'a remplacée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionReason {
    Write,
    Delete,
    Restore,
    /// Fichier déclaré réécrit au démarrage : la version gardée est celle laissée par le jeu.
    Start,
    #[serde(other)]
    Other,
}

/// Par où.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionChannel {
    Web,
    Sftp,
    Kuro,
    #[serde(other)]
    Other,
}

/// Réponse à `file.history.deleted`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeletedFiles {
    pub files: Vec<DeletedFile>,
}

/// Fichier absent du dossier, et sa dernière version gardée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeletedFile {
    pub path: String,
    pub version: FileVersion,
}
