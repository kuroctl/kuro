//! Fichiers d'un serveur (ADR-0015). Les chemins sont relatifs aux volumes :
//! `/<service>/<volume>/…`. Le daemon les valide lui-même (voir [`crate::limits::is_valid_path`]).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Contenu d'un dossier ; réponse `file.listing`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileList {
    pub server_id: Uuid,
    pub path: String,
}

/// Contenu texte d'un fichier ; réponse `file.content`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRead {
    pub server_id: Uuid,
    pub path: String,
}

/// Remplace (ou crée) un fichier texte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileWrite {
    pub server_id: Uuid,
    pub path: String,
    pub content: String,
    /// Empreinte du contenu lu : le daemon refuse (`conflict`) si le fichier a changé depuis.
    /// Absente : écriture inconditionnelle (création, écrasement voulu).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub if_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMkdir {
    pub server_id: Uuid,
    pub path: String,
}

/// Renomme ou déplace, dans les volumes du même service ou non.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRename {
    pub server_id: Uuid,
    pub from: String,
    pub to: String,
}

/// Supprime un fichier, un lien (pas sa cible) ou un dossier et tout son contenu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDelete {
    pub server_id: Uuid,
    pub path: String,
}

/// Début d'un envoi de fichier binaire (ADR-0020) : le daemon prépare un fichier temporaire,
/// puis reçoit les morceaux (`file.upload.chunk`). Quota et suspension s'appliquent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileUpload {
    pub server_id: Uuid,
    pub upload_id: Uuid,
    pub path: String,
    /// Taille annoncée ; un morceau qui la dépasse est refusé.
    pub size: u64,
}

/// Morceau d'un envoi, dans l'ordre (`offset`). Le dernier (`eof`) porte le SHA-256 calculé
/// par l'API : le fichier n'est renommé à sa place que si taille et empreinte correspondent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileUploadChunk {
    pub server_id: Uuid,
    pub upload_id: Uuid,
    pub offset: u64,
    /// Base64, au plus [`crate::limits::UPLOAD_CHUNK_BYTES`] octets une fois décodé.
    pub data: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub eof: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}
