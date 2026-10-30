//! Envoi de fichiers binaires (ADR-0020) : `file.upload` prépare un fichier temporaire sous un
//! volume, `file.upload.chunk` y ajoute les morceaux dans l'ordre. Au dernier, taille et SHA-256
//! sont vérifiés avant que le fichier prenne sa place ; sinon il disparaît. Un envoi abandonné
//! (aucun morceau depuis deux minutes) est jeté au prochain envoi.

use std::io::Write;
use std::time::{Duration, Instant};

use base64::Engine;
use kuro_protocol::api::{FileUpload, FileUploadChunk};
use kuro_protocol::limits::{MAX_UPLOAD_BYTES, UPLOAD_CHUNK_BYTES, path_segments};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{CommandError, Manager};
use crate::fs::volumes::NewFile;

/// Envois simultanés au plus par serveur.
const MAX_UPLOADS_PER_SERVER: usize = 4;
/// Sans morceau pendant ce délai, l'envoi est abandonné.
const IDLE: Duration = Duration::from_secs(120);

/// Envoi en cours.
pub(super) struct Upload {
    server_id: Uuid,
    pending: NewFile,
    hasher: Sha256,
    written: u64,
    size: u64,
    touched: Instant,
}

impl std::fmt::Debug for Upload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Upload")
            .field("server_id", &self.server_id)
            .field("written", &self.written)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl Manager {
    /// Oublie les envois sans morceau depuis [`IDLE`] : leur fichier temporaire disparaît avec
    /// eux. Appelé périodiquement, sans attendre un prochain envoi.
    pub(super) fn sweep_uploads(&self) {
        self.inner
            .uploads()
            .retain(|_, u| u.touched.elapsed() < IDLE);
    }

    /// Prépare l'envoi : chemin sous un volume, serveur ni suspendu ni occupé, quota respecté.
    pub async fn file_upload(&self, upload: FileUpload) -> Result<(), CommandError> {
        if upload.size > MAX_UPLOAD_BYTES {
            return Err(CommandError::TooLarge);
        }
        let segments: Vec<String> = path_segments(&upload.path)
            .ok_or(CommandError::InvalidPath)?
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        if segments.len() < 3
            || segments
                .last()
                .is_some_and(|name| name.starts_with(".kuro"))
        {
            return Err(CommandError::InvalidPath);
        }
        let borrowed: Vec<&str> = segments.iter().map(String::as_str).collect();
        let (deploy, volumes, owner) = self.upload_target(upload.server_id, &borrowed)?;
        self.check_quota(&deploy, upload.size).await?;
        {
            let mut uploads = self.inner.uploads();
            uploads.retain(|_, u| u.touched.elapsed() < IDLE);
            let running = uploads
                .values()
                .filter(|u| u.server_id == upload.server_id)
                .count();
            if running >= MAX_UPLOADS_PER_SERVER {
                return Err(CommandError::Busy);
            }
        }
        let pending = super::files::blocking(move || {
            let path: Vec<&str> = segments.iter().map(String::as_str).collect();
            volumes.mkdir_all(&path[..path.len() - 1], owner)?;
            volumes.create(&path, owner, 0o644)
        })
        .await?;
        self.inner.uploads().insert(
            upload.upload_id,
            Upload {
                server_id: upload.server_id,
                pending,
                hasher: Sha256::new(),
                written: 0,
                size: upload.size,
                touched: Instant::now(),
            },
        );
        Ok(())
    }

    /// Ajoute un morceau ; le dernier termine l'envoi. Toute erreur abandonne l'envoi.
    pub async fn file_upload_chunk(&self, chunk: FileUploadChunk) -> Result<(), CommandError> {
        let mut upload = self
            .inner
            .uploads()
            .remove(&chunk.upload_id)
            .filter(|u| u.server_id == chunk.server_id)
            .ok_or(CommandError::NotFound)?;
        if chunk.offset != upload.written {
            return Err(CommandError::Conflict);
        }
        let data = base64::engine::general_purpose::STANDARD
            .decode(chunk.data.as_bytes())
            .map_err(|_| CommandError::InvalidSpec("invalid base64".to_owned()))?;
        if data.len() > UPLOAD_CHUNK_BYTES
            || upload.written.saturating_add(data.len() as u64) > upload.size
        {
            return Err(CommandError::TooLarge);
        }
        let upload = tokio::task::spawn_blocking(move || {
            upload.pending.file.write_all(&data)?;
            upload.hasher.update(&data);
            upload.written += data.len() as u64;
            Ok::<_, std::io::Error>(upload)
        })
        .await
        .map_err(|e| CommandError::Internal(e.to_string()))?
        .map_err(|e| CommandError::Internal(e.to_string()))?;

        if !chunk.eof {
            let mut upload = upload;
            upload.touched = Instant::now();
            self.inner.uploads().insert(chunk.upload_id, upload);
            return Ok(());
        }
        let actual = kuro_runtime::tls::hex(&upload.hasher.clone().finalize());
        let expected = chunk.sha256.unwrap_or_default();
        if upload.written != upload.size || !actual.eq_ignore_ascii_case(&expected) {
            // Abandonné : le fichier temporaire disparaît avec `pending`.
            return Err(CommandError::Conflict);
        }
        let server_id = upload.server_id;
        super::files::blocking(move || upload.pending.commit()).await?;
        self.measure_later(server_id);
        Ok(())
    }
}
