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
    /// Qui écrit : repris dans l'historique du fichier (ADR-0049). Absent d'une API plus ancienne.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<FileAuthor>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<FileAuthor>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<FileAuthor>,
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

/// Auteur d'une écriture, gardé avec la version qu'elle remplace (ADR-0049) : un identifiant,
/// jamais une adresse e-mail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileAuthor {
    pub kind: AuthorKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorKind {
    Admin,
    Customer,
    Agent,
    System,
    #[serde(other)]
    Other,
}

/// Versions gardées d'un fichier, de la plus récente à la plus ancienne ; réponse
/// `file.versions`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileHistory {
    pub server_id: Uuid,
    pub path: String,
}

/// Contenu texte d'une version ; réponse `file.content` (refus `not_text` pour un binaire).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileVersionRead {
    pub server_id: Uuid,
    pub path: String,
    pub version: Uuid,
}

/// Remet une version à la place du fichier ; la version actuelle est gardée avant (la
/// restauration s'annule comme une autre écriture).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRestore {
    pub server_id: Uuid,
    pub path: String,
    pub version: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<FileAuthor>,
}

/// Fichiers d'un dossier supprimés dont une version est encore gardée ; réponse
/// `file.deleted_list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileHistoryDeleted {
    pub server_id: Uuid,
    /// Dossier (`/<service>/<volume>/…`) : seuls ses fichiers directs sont rendus.
    pub dir: String,
}
