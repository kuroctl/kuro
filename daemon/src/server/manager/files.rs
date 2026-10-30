//! Fichiers des serveurs (ADR-0015) : chemins validés contre la spécification déployée, puis
//! opérations confinées aux volumes ([`crate::fs::volumes`]).

use kuro_protocol::api::server::ServerDeploy;
use kuro_protocol::daemon::{FileContent, FileEntry, FileKind, FileListing};
use kuro_protocol::limits::{MAX_MESSAGE_BYTES, path_segments};
use uuid::Uuid;

use super::disk::ensure_active;
use super::{CommandError, Manager, owner_of};
use crate::fs::volumes::{Owner, VolumeError, Volumes};

/// Marge pour l'enveloppe autour du contenu d'un fichier.
const ENVELOPE_MARGIN: usize = 1024;
/// Entrées d'un dossier envoyées au plus ; au-delà, la liste est tronquée et signalée telle.
const MAX_LISTING_ENTRIES: usize = 5000;

impl From<VolumeError> for CommandError {
    fn from(error: VolumeError) -> Self {
        match error {
            VolumeError::InvalidPath => Self::InvalidPath,
            VolumeError::NotFound => Self::NotFound,
            VolumeError::AlreadyExists => Self::AlreadyExists,
            VolumeError::Conflict => Self::Conflict,
            VolumeError::NotText => Self::NotText,
            VolumeError::TooLarge => Self::TooLarge,
            VolumeError::Io(error) => Self::Internal(error.to_string()),
        }
    }
}

/// Chemin validé : segments, et volumes du serveur.
struct Target {
    deploy: ServerDeploy,
    volumes: Volumes,
}

impl Manager {
    pub async fn file_list(
        &self,
        server_id: Uuid,
        path: &str,
    ) -> Result<FileListing, CommandError> {
        let segments = segments(path)?;
        let target = self.target(server_id, false)?;
        let entries = match segments.as_slice() {
            [] => target
                .deploy
                .services
                .iter()
                .filter(|s| !s.volumes.is_empty())
                .map(|s| directory(&s.name))
                .collect(),
            [service] => target
                .deploy
                .services
                .iter()
                .find(|s| s.name == *service)
                .ok_or(CommandError::NotFound)?
                .volumes
                .iter()
                .map(|v| directory(&v.name))
                .collect(),
            _ => {
                check_volume(&target.deploy, &segments)?;
                let path = owned(&segments);
                blocking(move || target.volumes.list(&borrowed(&path))).await?
            }
        };
        // Un dossier de dizaines de milliers d'entrées dépasserait la taille d'un message.
        let truncated = entries.len() > MAX_LISTING_ENTRIES;
        let mut entries = entries;
        entries.truncate(MAX_LISTING_ENTRIES);
        Ok(FileListing { entries, truncated })
    }

    pub async fn file_read(
        &self,
        server_id: Uuid,
        path: &str,
    ) -> Result<FileContent, CommandError> {
        let segments = segments(path)?;
        let target = self.target(server_id, false)?;
        check_volume(&target.deploy, &segments)?;
        let path = owned(&segments);
        let content = blocking(move || target.volumes.read(&borrowed(&path))).await?;
        // Un texte plein de caractères à échapper peut dépasser la taille d'un message.
        let encoded = serde_json::to_string(&content.content)
            .map_err(|e| CommandError::Internal(e.to_string()))?;
        if encoded.len() + ENVELOPE_MARGIN > MAX_MESSAGE_BYTES {
            return Err(CommandError::TooLarge);
        }
        Ok(content)
    }

    pub async fn file_write(
        &self,
        server_id: Uuid,
        path: &str,
        content: String,
        if_sha256: Option<String>,
    ) -> Result<(), CommandError> {
        let segments = segments(path)?;
        below_volume(&segments)?;
        let target = self.target(server_id, true)?;
        let owner = check_volume(&target.deploy, &segments)?;
        self.check_quota(&target.deploy, content.len() as u64)
            .await?;
        let path = owned(&segments);
        blocking(move || {
            target
                .volumes
                .write(&borrowed(&path), &content, if_sha256.as_deref(), owner)
        })
        .await?;
        self.measure_later(server_id);
        Ok(())
    }

    pub async fn file_mkdir(&self, server_id: Uuid, path: &str) -> Result<(), CommandError> {
        let segments = segments(path)?;
        below_volume(&segments)?;
        let target = self.target(server_id, true)?;
        let owner = check_volume(&target.deploy, &segments)?;
        self.check_quota(&target.deploy, 0).await?;
        let path = owned(&segments);
        blocking(move || target.volumes.mkdir(&borrowed(&path), owner)).await
    }

    pub async fn file_rename(
        &self,
        server_id: Uuid,
        from: &str,
        to: &str,
    ) -> Result<(), CommandError> {
        let from = segments(from)?;
        let to = segments(to)?;
        below_volume(&from)?;
        below_volume(&to)?;
        let target = self.target(server_id, true)?;
        check_volume(&target.deploy, &from)?;
        check_volume(&target.deploy, &to)?;
        let (from, to) = (owned(&from), owned(&to));
        blocking(move || target.volumes.rename(&borrowed(&from), &borrowed(&to))).await
    }

    pub async fn file_delete(&self, server_id: Uuid, path: &str) -> Result<(), CommandError> {
        let segments = segments(path)?;
        below_volume(&segments)?;
        let target = self.target(server_id, true)?;
        check_volume(&target.deploy, &segments)?;
        let path = owned(&segments);
        blocking(move || target.volumes.delete(&borrowed(&path))).await?;
        self.measure_later(server_id);
        Ok(())
    }

    /// Destination d'un envoi : volume existant du serveur, et propriétaire des créations.
    pub(super) fn upload_target(
        &self,
        server_id: Uuid,
        segments: &[&str],
    ) -> Result<(ServerDeploy, Volumes, Owner), CommandError> {
        let target = self.target(server_id, true)?;
        let owner = check_volume(&target.deploy, segments)?;
        Ok((target.deploy, target.volumes, owner))
    }

    /// Serveur connu du nœud et pas suspendu ; une modification attend la fin d'un
    /// déploiement ou d'une suppression.
    fn target(&self, server_id: Uuid, write: bool) -> Result<Target, CommandError> {
        if write && self.is_busy(server_id) {
            return Err(CommandError::Busy);
        }
        let deploy = self.spec(server_id)?;
        ensure_active(&deploy)?;
        Ok(Target {
            deploy,
            volumes: Volumes::new(self.dir(server_id).root()),
        })
    }
}

fn segments(path: &str) -> Result<Vec<&str>, CommandError> {
    path_segments(path).ok_or(CommandError::InvalidPath)
}

/// Les services et leurs volumes sont des points de montage : on ne modifie qu'en dessous.
fn below_volume(segments: &[&str]) -> Result<(), CommandError> {
    if segments.len() < 3 {
        return Err(CommandError::InvalidPath);
    }
    Ok(())
}

/// Service et volume existent ; retourne l'utilisateur du service, propriétaire des créations.
fn check_volume(deploy: &ServerDeploy, segments: &[&str]) -> Result<Owner, CommandError> {
    let [service, volume, ..] = segments else {
        return Err(CommandError::InvalidPath);
    };
    let known = deploy
        .services
        .iter()
        .find(|s| s.name == *service)
        .is_some_and(|s| s.volumes.iter().any(|v| v.name == *volume));
    if !known {
        return Err(CommandError::NotFound);
    }
    let (uid, gid) =
        owner_of(deploy, service).map_err(|e| CommandError::Internal(e.to_string()))?;
    Ok(Owner { uid, gid })
}

fn directory(name: &str) -> FileEntry {
    FileEntry {
        name: name.to_owned(),
        kind: FileKind::Directory,
        size: 0,
        modified_ms: 0,
    }
}

fn owned(segments: &[&str]) -> Vec<String> {
    segments.iter().map(|s| (*s).to_owned()).collect()
}

fn borrowed(segments: &[String]) -> Vec<&str> {
    segments.iter().map(String::as_str).collect()
}

/// Accès disque hors du runtime asynchrone : la boucle du lien n'est pas bloquée.
pub(super) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, VolumeError> + Send + 'static,
) -> Result<T, CommandError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| CommandError::Internal(e.to_string()))?
        .map_err(CommandError::from)
}
