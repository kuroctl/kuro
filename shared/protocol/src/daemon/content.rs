//! Événements du daemon sur le contenu des serveurs (ADR-0020).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Fin d'un `content.install`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentDone {
    pub server_id: Uuid,
    pub operation_id: Uuid,
    #[serde(flatten)]
    pub outcome: ContentOutcome,
    /// Paquets extraits (installation avec `extract`) ; vide sinon.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub packages: Vec<ExtractedPackage>,
}

/// Paquet extrait d'une archive, et sa description (bornée par [`MAX_DESCRIPTION_BYTES`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractedPackage {
    /// Fichier téléchargé dont il vient (`file_name` du `content.install`).
    pub file_name: String,
    /// Dossier choisi par sa route, et nom du dossier créé dedans.
    pub dir: String,
    pub name: String,
    pub description: serde_json::Value,
}

/// Taille maximale du fichier de description d'un paquet.
pub const MAX_DESCRIPTION_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ContentOutcome {
    Ok,
    Failed {
        /// Cause, sans secret.
        reason: String,
    },
}
