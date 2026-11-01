//! Fichiers des serveurs (ADR-0015) : chemins validés contre la spécification déployée, puis
//! opérations confinées aux volumes ([`crate::fs::volumes`]).

use kuro_protocol::api::server::ServerDeploy;
use kuro_protocol::api::{FileAuthor, HistoryPolicy};
use kuro_protocol::daemon::{
    DeletedFiles, FileContent, FileEntry, FileKind, FileListing, FileVersions, VersionChannel,
    VersionReason,
};
use kuro_protocol::limits::{MAX_MESSAGE_BYTES, path_segments};
use uuid::Uuid;

use super::disk::ensure_active;
use super::{CommandError, Manager, owner_of};
use crate::fs::history::{Capture, History, HistoryError};
use crate::fs::volumes::{Owner, VolumeError, Volumes};
use crate::sftp::ipc::ServiceLayout;

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
        self.file_write_as(server_id, path, content, if_sha256, None)
            .await
    }

    /// Écriture de l'éditeur du panel ; la version remplacée est gardée (ADR-0049).
    pub async fn file_write_as(
        &self,
        server_id: Uuid,
        path: &str,
        content: String,
        if_sha256: Option<String>,
        author: Option<FileAuthor>,
    ) -> Result<(), CommandError> {
        let segments = segments(path)?;
        below_volume(&segments)?;
        let target = self.target(server_id, true)?;
        let owner = check_volume(&target.deploy, &segments)?;
        self.check_quota(&target.deploy, content.len() as u64)
            .await?;
        let path = owned(&segments);
        self.keep_version(server_id, &path, web(VersionReason::Write, author))
            .await;
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
        self.file_delete_as(server_id, path, None).await
    }

    /// Suppression depuis le panel ; un fichier supprimé garde sa dernière version (ADR-0049).
    pub async fn file_delete_as(
        &self,
        server_id: Uuid,
        path: &str,
        author: Option<FileAuthor>,
    ) -> Result<(), CommandError> {
        let segments = segments(path)?;
        below_volume(&segments)?;
        let target = self.target(server_id, true)?;
        check_volume(&target.deploy, &segments)?;
        let path = owned(&segments);
        self.keep_version(server_id, &path, web(VersionReason::Delete, author))
            .await;
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

impl Manager {
    /// Historique du serveur (ADR-0049).
    pub(crate) fn history(&self, server_id: Uuid) -> History {
        History::new(self.dir(server_id).root()).with_policy(self.history_policy())
    }

    /// Rétention en vigueur, transmise aussi au processus SFTP.
    pub fn history_policy(&self) -> HistoryPolicy {
        *self.inner.history_policy()
    }

    /// Nouvelle rétention (`hello.ack`, `node.settings`) : appliquée aussitôt à l'historique de
    /// chaque serveur, en tâche de fond. Une durée nulle efface tout.
    pub fn apply_history_policy(&self, policy: HistoryPolicy) {
        let changed = {
            let mut current = self.inner.history_policy();
            let changed = *current != policy;
            *current = policy;
            changed
        };
        if !changed {
            return;
        }
        let histories: Vec<History> = self
            .deployed()
            .into_iter()
            .map(|server_id| self.history(server_id))
            .collect();
        tokio::task::spawn_blocking(move || {
            for history in histories {
                if let Err(error) = history.purge() {
                    tracing::warn!(%error, "file history not purged");
                }
            }
        });
    }

    /// Fichiers supprimés d'un dossier dont une version est gardée.
    pub async fn file_history_deleted(
        &self,
        server_id: Uuid,
        dir: &str,
    ) -> Result<DeletedFiles, CommandError> {
        let segments = segments(dir)?;
        let target = self.target(server_id, false)?;
        check_volume(&target.deploy, &segments)?;
        let history = self.history(server_id);
        let dir = owned(&segments);
        let files =
            tokio::task::spawn_blocking(move || history.deleted(&target.volumes, &borrowed(&dir)))
                .await
                .map_err(|e| CommandError::Internal(e.to_string()))?
                .map_err(history_error)?;
        Ok(DeletedFiles { files })
    }

    /// Garde la version actuelle d'un fichier avant de la remplacer. Un échec de l'historique
    /// (disque plein…) est journalisé mais n'empêche pas l'écriture demandée.
    pub(super) async fn keep_version(&self, server_id: Uuid, path: &[String], capture: Capture) {
        let history = self.history(server_id);
        let volumes = Volumes::new(self.dir(server_id).root());
        let path = path.to_vec();
        let kept = tokio::task::spawn_blocking(move || {
            history.capture(&volumes, &borrowed(&path), capture)
        })
        .await;
        match kept {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::warn!(%server_id, %error, "file version not kept"),
            Err(error) => tracing::warn!(%server_id, %error, "file version not kept"),
        }
    }

    /// Versions gardées d'un fichier, de la plus récente à la plus ancienne.
    pub async fn file_history(
        &self,
        server_id: Uuid,
        path: &str,
    ) -> Result<FileVersions, CommandError> {
        let segments = segments(path)?;
        below_volume(&segments)?;
        let target = self.target(server_id, false)?;
        check_volume(&target.deploy, &segments)?;
        let history = self.history(server_id);
        let path = owned(&segments);
        let versions = tokio::task::spawn_blocking(move || history.versions(&borrowed(&path)))
            .await
            .map_err(|e| CommandError::Internal(e.to_string()))?
            .map_err(history_error)?;
        Ok(FileVersions { versions })
    }

    /// Contenu texte d'une version (comme `file.read` : UTF-8, taille d'un message).
    pub async fn file_version_read(
        &self,
        server_id: Uuid,
        path: &str,
        version: Uuid,
    ) -> Result<FileContent, CommandError> {
        let content = self.version_bytes(server_id, path, version).await?;
        let size = content.len() as u64;
        let sha256 = kuro_runtime::tls::hex(&<sha2::Sha256 as sha2::Digest>::digest(&content));
        let content = String::from_utf8(content).map_err(|_| CommandError::NotText)?;
        if content.len() > kuro_protocol::limits::MAX_TEXT_FILE_BYTES {
            return Err(CommandError::TooLarge);
        }
        Ok(FileContent {
            content,
            size,
            modified_ms: 0,
            sha256,
        })
    }

    async fn version_bytes(
        &self,
        server_id: Uuid,
        path: &str,
        version: Uuid,
    ) -> Result<Vec<u8>, CommandError> {
        let segments = segments(path)?;
        below_volume(&segments)?;
        let target = self.target(server_id, false)?;
        check_volume(&target.deploy, &segments)?;
        let history = self.history(server_id);
        let path = owned(&segments);
        tokio::task::spawn_blocking(move || history.read(&borrowed(&path), version))
            .await
            .map_err(|e| CommandError::Internal(e.to_string()))?
            .map_err(history_error)
    }

    /// Remet une version à la place du fichier ; la version actuelle est gardée avant, la
    /// restauration s'annule donc comme une autre écriture. Un texte garde le propriétaire et
    /// les droits du fichier remplacé ; un binaire est réécrit en `0644`.
    pub async fn file_restore(
        &self,
        server_id: Uuid,
        path: &str,
        version: Uuid,
        author: Option<FileAuthor>,
    ) -> Result<(), CommandError> {
        let content = self.version_bytes(server_id, path, version).await?;
        let segments = segments(path)?;
        let target = self.target(server_id, true)?;
        let owner = check_volume(&target.deploy, &segments)?;
        self.check_quota(&target.deploy, content.len() as u64)
            .await?;
        let path = owned(&segments);
        self.keep_version(server_id, &path, web(VersionReason::Restore, author))
            .await;
        blocking(move || {
            let path = borrowed(&path);
            target.volumes.mkdir_all(&path[..path.len() - 1], owner)?;
            match std::str::from_utf8(&content) {
                Ok(text) if text.len() <= kuro_protocol::limits::MAX_TEXT_FILE_BYTES => {
                    target.volumes.write(&path, text, None, owner)
                }
                _ => {
                    use std::io::Write;
                    let mut file = target.volumes.create(&path, owner, 0o644)?;
                    file.file.write_all(&content)?;
                    file.commit()
                }
            }
        })
        .await?;
        self.measure_later(server_id);
        Ok(())
    }

    /// Services et volumes visibles d'un serveur actif, pour une session SFTP (ADR-0047).
    pub fn sftp_layout(&self, server_id: Uuid) -> Result<Vec<ServiceLayout>, CommandError> {
        let deploy = self.spec(server_id)?;
        ensure_active(&deploy)?;
        deploy
            .services
            .iter()
            .filter(|service| !service.volumes.is_empty())
            .map(|service| {
                let (uid, gid) = owner_of(&deploy, &service.name)
                    .map_err(|e| CommandError::Internal(e.to_string()))?;
                Ok(ServiceLayout {
                    name: service.name.clone(),
                    volumes: service.volumes.iter().map(|v| v.name.clone()).collect(),
                    uid,
                    gid,
                })
            })
            .collect()
    }

    /// Écriture SFTP permise, aux mêmes conditions que le panel : serveur actif, aucun
    /// déploiement ni suppression en cours, quota non atteint.
    pub async fn sftp_write_check(&self, server_id: Uuid) -> Result<(), CommandError> {
        let target = self.target(server_id, true)?;
        self.check_quota(&target.deploy, 0).await
    }

    /// Des fichiers ont été écrits par SFTP : nouvelle mesure de l'espace disque.
    pub fn sftp_written(&self, server_id: Uuid) {
        self.measure_later(server_id);
    }
}

/// Capture faite pour le panel (requête de l'API).
fn web(reason: VersionReason, author: Option<FileAuthor>) -> Capture {
    Capture {
        reason,
        channel: VersionChannel::Web,
        author,
    }
}

fn history_error(error: HistoryError) -> CommandError {
    match error {
        HistoryError::NotFound => CommandError::NotFound,
        HistoryError::Volume(error) => CommandError::from(error),
        other => CommandError::Internal(other.to_string()),
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
