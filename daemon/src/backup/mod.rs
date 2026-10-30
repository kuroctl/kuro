//! Sauvegardes des serveurs (ADR-0018) : archives `tar.zst` des volumes, gardées sur le nœud hors
//! des volumes, dans `<dossier du serveur>/backups/`.
//!
//! - `<id>.tar.zst` : l'archive, écrite sous un nom temporaire puis renommée ;
//! - `<id>.json` : taille et sha256, écrit en dernier. Une archive sans ce fichier est incomplète.

pub mod archive;
pub mod pattern;

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use kuro_protocol::limits::BACKUP_CHUNK_BYTES;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Préfixe d'une archive en cours d'écriture.
const TEMP_PREFIX: &str = ".kuro-tmp-";
const ARCHIVE_EXT: &str = ".tar.zst";

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("no such backup")]
    NotFound,
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("volume: {0}")]
    Volume(#[from] crate::fs::volumes::VolumeError),
    #[error("invalid backup metadata: {0}")]
    Meta(#[from] serde_json::Error),
}

/// Sauvegarde terminée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    pub size: u64,
    pub sha256: String,
}

/// Sauvegardes d'un serveur sur le disque du nœud.
#[derive(Debug, Clone)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(server_root: &Path) -> Self {
        Self {
            dir: server_root.join("backups"),
        }
    }

    fn archive(&self, id: Uuid) -> PathBuf {
        self.dir.join(format!("{id}{ARCHIVE_EXT}"))
    }

    fn meta(&self, id: Uuid) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    fn temp(&self, id: Uuid) -> PathBuf {
        self.dir.join(format!("{TEMP_PREFIX}{id}{ARCHIVE_EXT}"))
    }

    /// Fichier où écrire l'archive ; elle n'existe qu'après [`Store::finish`].
    pub fn begin(&self, id: Uuid) -> Result<File, BackupError> {
        create_private_dir(&self.dir)?;
        Ok(File::create(self.temp(id))?)
    }

    /// L'archive prend son nom, puis ses métadonnées la rendent visible.
    pub fn finish(&self, id: Uuid, meta: &Meta) -> Result<(), BackupError> {
        std::fs::rename(self.temp(id), self.archive(id))?;
        let temp = self.dir.join(format!("{TEMP_PREFIX}{id}.json"));
        let mut file = File::create(&temp)?;
        file.write_all(&serde_json::to_vec(meta)?)?;
        file.sync_all()?;
        std::fs::rename(temp, self.meta(id))?;
        Ok(())
    }

    /// Archive abandonnée (échec de l'archivage).
    pub fn abort(&self, id: Uuid) {
        let _ = std::fs::remove_file(self.temp(id));
    }

    pub fn get(&self, id: Uuid) -> Result<Meta, BackupError> {
        match std::fs::read(self.meta(id)) {
            Ok(bytes) if self.archive(id).is_file() => Ok(serde_json::from_slice(&bytes)?),
            Ok(_) => Err(BackupError::NotFound),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Err(BackupError::NotFound)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Sauvegardes terminées.
    pub fn list(&self) -> Result<Vec<(Uuid, Meta)>, BackupError> {
        let mut found = Vec::new();
        for (id, _) in self.entries()? {
            match self.get(id) {
                Ok(meta) => found.push((id, meta)),
                Err(BackupError::NotFound) => {}
                Err(error) => return Err(error),
            }
        }
        found.sort_by_key(|(id, _)| *id);
        found.dedup_by_key(|(id, _)| *id);
        Ok(found)
    }

    /// Supprime l'archive et ses métadonnées ; absente, rien à faire.
    pub fn delete(&self, id: Uuid) -> Result<(), BackupError> {
        for path in [self.meta(id), self.archive(id)] {
            match std::fs::remove_file(&path) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    return Err(error.into());
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Archive ouverte en lecture (restauration).
    pub fn open(&self, id: Uuid) -> Result<File, BackupError> {
        self.get(id)?;
        Ok(File::open(self.archive(id))?)
    }

    /// Au plus [`BACKUP_CHUNK_BYTES`] à partir de `offset`, et si la fin est atteinte.
    pub fn read(&self, id: Uuid, offset: u64) -> Result<(Vec<u8>, bool), BackupError> {
        let meta = self.get(id)?;
        let mut file = File::open(self.archive(id))?;
        file.seek(SeekFrom::Start(offset))?;
        let mut data = Vec::with_capacity(BACKUP_CHUNK_BYTES);
        file.take(BACKUP_CHUNK_BYTES as u64)
            .read_to_end(&mut data)?;
        let eof = offset.saturating_add(data.len() as u64) >= meta.size;
        Ok((data, eof))
    }

    /// Supprime les restes d'archivages interrompus (daemon arrêté), sauf celui en cours.
    pub fn clean(&self, current: Option<Uuid>) -> Result<(), BackupError> {
        for (id, path) in self.entries()? {
            if Some(id) == current {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            let orphan = name.starts_with(TEMP_PREFIX)
                || (name.ends_with(ARCHIVE_EXT) && !self.meta(id).exists())
                || (name.ends_with(".json") && !self.archive(id).exists());
            if orphan {
                let _ = std::fs::remove_file(&path);
            }
        }
        Ok(())
    }

    /// Fichiers du dossier dont le nom porte un identifiant de sauvegarde.
    fn entries(&self) -> Result<Vec<(Uuid, PathBuf)>, BackupError> {
        let read = match std::fs::read_dir(&self.dir) {
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut entries = Vec::new();
        for entry in read {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let bare = name.strip_prefix(TEMP_PREFIX).unwrap_or(name);
            let id = bare
                .strip_suffix(ARCHIVE_EXT)
                .or_else(|| bare.strip_suffix(".json"))
                .and_then(|id| Uuid::parse_str(id).ok());
            if let Some(id) = id {
                entries.push((id, entry.path()));
            }
        }
        Ok(entries)
    }
}

/// Dossier lisible par le seul daemon : les sauvegardes peuvent contenir des secrets du jeu.
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
