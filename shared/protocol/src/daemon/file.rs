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
