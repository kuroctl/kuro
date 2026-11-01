//! File locale des événements d'audit SFTP (ADR-0047), en ajout seul : un lot par fichier, écrit
//! avant d'être accepté, effacé une fois acquitté par l'API. Elle survit à une coupure du lien
//! et à un redémarrage du daemon. Pleine, elle refuse : le processus SFTP refuse alors les
//! écritures, aucune écriture sans trace.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use kuro_protocol::daemon::sftp::SftpEvent;
use kuro_protocol::limits::MAX_SFTP_AUDIT_BATCH;
use tokio::sync::Notify;
use uuid::Uuid;

/// Événements gardés au plus.
pub const MAX_EVENTS: usize = 10_000;
/// Octets gardés au plus.
pub const MAX_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum SpoolError {
    #[error("the sftp audit spool is full")]
    Full,
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("unreadable spool file: {0}")]
    Corrupt(#[from] serde_json::Error),
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> SpoolError + '_ {
    move |source| SpoolError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[derive(Debug, Default)]
struct Usage {
    events: usize,
    bytes: u64,
}

#[derive(Debug)]
pub struct Spool {
    dir: PathBuf,
    usage: Mutex<Usage>,
    /// Réveille l'envoi dès qu'un lot arrive.
    pub(crate) arrived: Notify,
}

/// Lot en attente : son fichier et ses événements.
#[derive(Debug)]
pub struct Batch {
    path: PathBuf,
    bytes: u64,
    pub events: Vec<SftpEvent>,
}

impl Spool {
    /// Ouvre la file (dossier créé en `0700`) et compte ce qui y attend déjà.
    pub fn open(dir: &Path) -> Result<Self, SpoolError> {
        create_private_dir(dir)?;
        let spool = Self {
            dir: dir.to_path_buf(),
            usage: Mutex::new(Usage::default()),
            arrived: Notify::new(),
        };
        let mut usage = Usage::default();
        for path in spool.files()? {
            let batch = spool.read(&path)?;
            usage.events += batch.events.len();
            usage.bytes += batch.bytes;
        }
        *spool.lock() = usage;
        Ok(spool)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Usage> {
        self.usage
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn pending(&self) -> usize {
        self.lock().events
    }

    /// Écrit les événements sur disque (lots de [`MAX_SFTP_AUDIT_BATCH`] au plus) avant de rendre
    /// la main ; refuse tout si la file déborderait.
    pub fn append(&self, events: Vec<SftpEvent>) -> Result<(), SpoolError> {
        if events.is_empty() {
            return Ok(());
        }
        let chunks: Vec<Vec<u8>> = events
            .chunks(MAX_SFTP_AUDIT_BATCH)
            .map(serde_json::to_vec)
            .collect::<Result<_, _>>()?;
        let bytes: u64 = chunks.iter().map(|chunk| chunk.len() as u64).sum();
        {
            let usage = self.lock();
            if usage.events + events.len() > MAX_EVENTS || usage.bytes + bytes > MAX_BYTES {
                return Err(SpoolError::Full);
            }
        }
        for chunk in &chunks {
            let name = format!("{}.json", Uuid::now_v7());
            let temp = self.dir.join(format!(".{name}.tmp"));
            std::fs::write(&temp, chunk).map_err(io(&temp))?;
            let path = self.dir.join(name);
            std::fs::rename(&temp, &path).map_err(io(&path))?;
        }
        let mut usage = self.lock();
        usage.events += events.len();
        usage.bytes += bytes;
        drop(usage);
        self.arrived.notify_one();
        Ok(())
    }

    /// Plus ancien lot en attente (les noms UUID v7 sont ordonnés dans le temps).
    pub fn oldest(&self) -> Result<Option<Batch>, SpoolError> {
        match self.files()?.into_iter().next() {
            Some(path) => self.read(&path).map(Some),
            None => Ok(None),
        }
    }

    /// Lot acquitté par l'API : oublié.
    pub fn acknowledge(&self, batch: Batch) -> Result<(), SpoolError> {
        std::fs::remove_file(&batch.path).map_err(io(&batch.path))?;
        let mut usage = self.lock();
        usage.events = usage.events.saturating_sub(batch.events.len());
        usage.bytes = usage.bytes.saturating_sub(batch.bytes);
        Ok(())
    }

    fn files(&self) -> Result<Vec<PathBuf>, SpoolError> {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&self.dir)
            .map_err(io(&self.dir))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension().is_some_and(|ext| ext == "json")
                    && !path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with('.'))
            })
            .collect();
        files.sort();
        Ok(files)
    }

    fn read(&self, path: &Path) -> Result<Batch, SpoolError> {
        let bytes = std::fs::read(path).map_err(io(path))?;
        Ok(Batch {
            path: path.to_path_buf(),
            bytes: bytes.len() as u64,
            events: serde_json::from_slice(&bytes)?,
        })
    }
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> Result<(), SpoolError> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(io(dir))
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> Result<(), SpoolError> {
    std::fs::create_dir_all(dir).map_err(io(dir))
}

#[cfg(test)]
mod tests {
    use kuro_protocol::daemon::sftp::{SftpEventKind, SftpPrincipal};

    use super::*;

    fn event() -> SftpEvent {
        SftpEvent {
            event_id: Uuid::now_v7(),
            occurred_at_ms: 0,
            session_id: Uuid::nil(),
            server_id: Uuid::nil(),
            principal: SftpPrincipal::Admin(Uuid::nil()),
            client_ip: "127.0.0.1".to_owned(),
            client_version: "SSH-2.0-test".to_owned(),
            event: SftpEventKind::AccessDenied {
                permission: "servers.files.write".to_owned(),
            },
        }
    }

    #[test]
    fn batches_survive_a_restart_until_acknowledged() {
        let dir = tempfile::tempdir().unwrap();
        let spool = Spool::open(dir.path()).unwrap();
        spool.append(vec![event(), event()]).unwrap();
        spool.append(vec![event()]).unwrap();
        assert_eq!(spool.pending(), 3);

        let reopened = Spool::open(dir.path()).unwrap();
        assert_eq!(reopened.pending(), 3);
        let first = reopened.oldest().unwrap().unwrap();
        assert_eq!(first.events.len(), 2);
        reopened.acknowledge(first).unwrap();
        assert_eq!(reopened.pending(), 1);
        let second = reopened.oldest().unwrap().unwrap();
        reopened.acknowledge(second).unwrap();
        assert!(reopened.oldest().unwrap().is_none());
    }

    #[test]
    fn a_full_spool_refuses_everything() {
        let dir = tempfile::tempdir().unwrap();
        let spool = Spool::open(dir.path()).unwrap();
        let many: Vec<SftpEvent> = (0..MAX_EVENTS).map(|_| event()).collect();
        spool.append(many).unwrap();
        assert!(matches!(spool.append(vec![event()]), Err(SpoolError::Full)));
        assert_eq!(spool.pending(), MAX_EVENTS);
    }
}
