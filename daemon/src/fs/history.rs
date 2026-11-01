//! Historique des fichiers d'un serveur (ADR-0049) : avant chaque écriture faite par Kuro (panel,
//! SFTP, restauration, réécriture des fichiers déclarés au démarrage), la version précédente est
//! gardée dans `<serveur>/.kuro/history/`, hors des volumes : jamais visible du conteneur ni du
//! SFTP.
//!
//! - `objects/<sha256>.zst` : contenus, dédoublonnés et compressés ;
//! - `index.jsonl` : une version par ligne, en ajout seul (réécrit seulement par la purge) ;
//! - `lock` : verrou exclusif, partagé avec le processus SFTP, qui écrit lui aussi l'historique.
//!
//! Le fichier est lu par [`Volumes`] : aucun lien suivi. Au-delà de [`MAX_VERSION_BYTES`], rien
//! n'est gardé. La purge suit la politique du projet ([`HistoryPolicy`], 30 jours et 100 Mio par
//! défaut) ; une durée nulle coupe l'historique et efface ce qui était gardé.

use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use kuro_protocol::api::{FileAuthor, HistoryPolicy};
use kuro_protocol::daemon::{DeletedFile, FileKind, FileVersion, VersionChannel, VersionReason};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::volumes::{VolumeError, Volumes};

/// Plus gros fichier gardé (configurations, scripts, petits datapacks).
pub const MAX_VERSION_BYTES: u64 = 2 * 1024 * 1024;
/// Fichiers supprimés rendus au plus pour un dossier.
const MAX_DELETED: usize = 200;

const ZSTD_LEVEL: i32 = 3;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error("no such version")]
    NotFound,
    #[error(transparent)]
    Volume(#[from] VolumeError),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("unreadable history index: {0}")]
    Index(#[from] serde_json::Error),
}

/// Ligne de `index.jsonl`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    id: Uuid,
    /// `/<service>/<volume>/…`, comme l'explorateur du panel.
    path: String,
    at_ms: i64,
    size: u64,
    sha256: String,
    reason: VersionReason,
    channel: VersionChannel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    author: Option<FileAuthor>,
}

impl From<Entry> for FileVersion {
    fn from(entry: Entry) -> Self {
        Self {
            id: entry.id,
            at_ms: entry.at_ms,
            size: entry.size,
            sha256: entry.sha256,
            reason: entry.reason,
            channel: entry.channel,
            author: entry.author,
        }
    }
}

/// Contexte d'une capture : ce qui remplace la version, par où, et qui.
#[derive(Debug, Clone, Copy)]
pub struct Capture {
    pub reason: VersionReason,
    pub channel: VersionChannel,
    pub author: Option<FileAuthor>,
}

#[derive(Debug, Clone)]
pub struct History {
    dir: PathBuf,
    policy: HistoryPolicy,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

fn display(segments: &[&str]) -> String {
    format!("/{}", segments.join("/"))
}

impl History {
    /// Historique du serveur dont le dossier est `server_root`.
    pub fn new(server_root: &Path) -> Self {
        Self {
            dir: server_root.join(".kuro").join("history"),
            policy: HistoryPolicy::default(),
        }
    }

    /// Rétention réglée dans les paramètres du projet.
    pub fn with_policy(mut self, policy: HistoryPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Applique la rétention sans nouvelle version (réglage changé). Sans historique, rien.
    pub fn purge(&self) -> Result<(), HistoryError> {
        if !self.index().exists() {
            return Ok(());
        }
        let _lock = self.lock()?;
        let entries = self.read_index()?;
        self.prune(entries, now_ms())
    }

    /// Fichiers directement dans `dir` (`[service, volume, …]`) qui n'existent plus et dont une
    /// version est gardée : la plus récente de chacun, du plus récemment supprimé au plus ancien.
    pub fn deleted(
        &self,
        volumes: &Volumes,
        dir: &[&str],
    ) -> Result<Vec<DeletedFile>, HistoryError> {
        let prefix = format!("{}/", display(dir));
        let latest: Vec<Entry> = {
            let _lock = self.lock()?;
            let mut latest: std::collections::HashMap<String, Entry> =
                std::collections::HashMap::new();
            for entry in self.read_index()? {
                let direct = entry
                    .path
                    .strip_prefix(&prefix)
                    .is_some_and(|name| !name.is_empty() && !name.contains('/'));
                if direct {
                    latest.insert(entry.path.clone(), entry);
                }
            }
            latest.into_values().collect()
        };
        let mut files = Vec::new();
        for entry in latest {
            let name = entry.path.rsplit('/').next().unwrap_or_default().to_owned();
            let mut segments: Vec<&str> = dir.to_vec();
            segments.push(&name);
            if matches!(volumes.stat(&segments), Err(VolumeError::NotFound)) {
                files.push(DeletedFile {
                    path: entry.path.clone(),
                    version: FileVersion::from(entry),
                });
            }
        }
        files.sort_by_key(|file| std::cmp::Reverse(file.version.at_ms));
        files.truncate(MAX_DELETED);
        Ok(files)
    }

    fn index(&self) -> PathBuf {
        self.dir.join("index.jsonl")
    }

    fn object(&self, sha256: &str) -> PathBuf {
        self.dir.join("objects").join(format!("{sha256}.zst"))
    }

    /// Verrou exclusif sur l'historique, relâché à la fermeture du fichier rendu.
    fn lock(&self) -> Result<std::fs::File, HistoryError> {
        std::fs::create_dir_all(self.dir.join("objects"))?;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.dir.join("lock"))?;
        lock.lock()?;
        Ok(lock)
    }

    /// Garde la version actuelle de `segments` (`[service, volume, …]`) avant qu'elle soit
    /// remplacée. Sans effet si le fichier n'existe pas, n'est pas un fichier ordinaire, dépasse
    /// [`MAX_VERSION_BYTES`], ou n'a pas changé depuis la dernière version gardée.
    pub fn capture(
        &self,
        volumes: &Volumes,
        segments: &[&str],
        capture: Capture,
    ) -> Result<(), HistoryError> {
        if self.policy.days == 0 {
            return Ok(());
        }
        let entry = match volumes.stat(segments) {
            Ok(entry) => entry,
            Err(VolumeError::NotFound) => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        if entry.kind != FileKind::File || entry.size > MAX_VERSION_BYTES {
            return Ok(());
        }
        let mut content = Vec::new();
        volumes
            .open(segments)?
            .take(MAX_VERSION_BYTES + 1)
            .read_to_end(&mut content)?;
        if content.len() as u64 > MAX_VERSION_BYTES {
            return Ok(());
        }
        self.keep(&display(segments), &content, capture, now_ms())
    }

    fn keep(
        &self,
        path: &str,
        content: &[u8],
        capture: Capture,
        at_ms: i64,
    ) -> Result<(), HistoryError> {
        let sha256 = hex(&Sha256::digest(content));
        let _lock = self.lock()?;
        let mut entries = self.read_index()?;
        let unchanged = entries
            .iter()
            .rev()
            .find(|entry| entry.path == path)
            .is_some_and(|entry| entry.sha256 == sha256);
        if unchanged {
            return Ok(());
        }
        let object = self.object(&sha256);
        if !object.exists() {
            let compressed = zstd::encode_all(content, ZSTD_LEVEL)?;
            let temp = object.with_extension("zst.tmp");
            std::fs::write(&temp, compressed)?;
            std::fs::rename(&temp, &object)?;
        }
        let entry = Entry {
            id: Uuid::now_v7(),
            path: path.to_owned(),
            at_ms,
            size: content.len() as u64,
            sha256,
            reason: capture.reason,
            channel: capture.channel,
            author: capture.author,
        };
        let mut line = serde_json::to_vec(&entry)?;
        line.push(b'\n');
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.index())?
            .write_all(&line)?;
        entries.push(entry);
        self.prune(entries, at_ms)
    }

    /// Retire les versions trop anciennes, puis les plus anciennes jusqu'à tenir dans la place
    /// permise ; efface les contenus que plus aucune version ne désigne.
    fn prune(&self, entries: Vec<Entry>, now_ms: i64) -> Result<(), HistoryError> {
        let oldest = if self.policy.days == 0 {
            i64::MAX
        } else {
            now_ms - i64::from(self.policy.days) * DAY_MS
        };
        let max_bytes = u64::from(self.policy.max_mb) * 1024 * 1024;
        let mut kept: Vec<Entry> = entries
            .iter()
            .filter(|e| e.at_ms >= oldest)
            .cloned()
            .collect();
        let size_of = |sha256: &str| {
            std::fs::metadata(self.object(sha256))
                .map(|meta| meta.len())
                .unwrap_or(0)
        };
        loop {
            let objects: HashSet<&str> = kept.iter().map(|e| e.sha256.as_str()).collect();
            let total: u64 = objects.iter().map(|sha| size_of(sha)).sum();
            if total <= max_bytes || kept.is_empty() {
                break;
            }
            kept.remove(0);
        }
        if kept.len() == entries.len() {
            return Ok(());
        }
        let mut text = Vec::new();
        for entry in &kept {
            text.extend(serde_json::to_vec(entry)?);
            text.push(b'\n');
        }
        let temp = self.index().with_extension("jsonl.tmp");
        std::fs::write(&temp, text)?;
        std::fs::rename(&temp, self.index())?;
        let referenced: HashSet<&str> = kept.iter().map(|e| e.sha256.as_str()).collect();
        for entry in &entries {
            if !referenced.contains(entry.sha256.as_str()) {
                let _ = std::fs::remove_file(self.object(&entry.sha256));
            }
        }
        Ok(())
    }

    fn read_index(&self) -> Result<Vec<Entry>, HistoryError> {
        let text = match std::fs::read_to_string(self.index()) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut entries = Vec::new();
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            match serde_json::from_str(line) {
                Ok(entry) => entries.push(entry),
                // Ligne tronquée par un arrêt brutal : ignorée, les autres restent lisibles.
                Err(error) => tracing::warn!(%error, "unreadable history line skipped"),
            }
        }
        Ok(entries)
    }

    /// Versions d'un fichier, de la plus récente à la plus ancienne.
    pub fn versions(&self, segments: &[&str]) -> Result<Vec<FileVersion>, HistoryError> {
        let path = display(segments);
        let _lock = self.lock()?;
        let mut versions: Vec<FileVersion> = self
            .read_index()?
            .into_iter()
            .filter(|entry| entry.path == path)
            .map(FileVersion::from)
            .collect();
        versions.reverse();
        Ok(versions)
    }

    /// Contenu d'une version du fichier.
    pub fn read(&self, segments: &[&str], version: Uuid) -> Result<Vec<u8>, HistoryError> {
        let path = display(segments);
        let _lock = self.lock()?;
        let entry = self
            .read_index()?
            .into_iter()
            .find(|entry| entry.id == version && entry.path == path)
            .ok_or(HistoryError::NotFound)?;
        let compressed = std::fs::read(self.object(&entry.sha256))?;
        let mut content = Vec::new();
        zstd::Decoder::new(compressed.as_slice())?
            .take(MAX_VERSION_BYTES + 1)
            .read_to_end(&mut content)?;
        Ok(content)
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const WEB: Capture = Capture {
        reason: VersionReason::Write,
        channel: VersionChannel::Web,
        author: None,
    };

    fn server() -> (tempfile::TempDir, Volumes, History) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("volumes/app/data")).unwrap();
        let volumes = Volumes::new(dir.path());
        let history = History::new(dir.path());
        (dir, volumes, history)
    }

    fn put(dir: &tempfile::TempDir, name: &str, content: &[u8]) {
        std::fs::write(dir.path().join("volumes/app/data").join(name), content).unwrap();
    }

    #[test]
    fn each_change_is_kept_once_and_read_back() {
        let (dir, volumes, history) = server();
        let path = ["app", "data", "server.properties"];
        put(&dir, "server.properties", b"motd=one");
        history.capture(&volumes, &path, WEB).unwrap();
        // Rien n'a changé : pas de nouvelle version.
        history.capture(&volumes, &path, WEB).unwrap();
        put(&dir, "server.properties", b"motd=two");
        history.capture(&volumes, &path, WEB).unwrap();

        let versions = history.versions(&path).unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(history.read(&path, versions[0].id).unwrap(), b"motd=two");
        assert_eq!(history.read(&path, versions[1].id).unwrap(), b"motd=one");
        // Une version d'un autre fichier n'est pas lisible sous ce chemin.
        assert!(matches!(
            history.read(&["app", "data", "other"], versions[0].id),
            Err(HistoryError::NotFound)
        ));
    }

    #[test]
    fn missing_large_or_special_files_are_not_kept() {
        let (dir, volumes, history) = server();
        history
            .capture(&volumes, &["app", "data", "absent"], WEB)
            .unwrap();
        let large = vec![b'x'; usize::try_from(MAX_VERSION_BYTES).unwrap() + 1];
        put(&dir, "world.dat", &large);
        history
            .capture(&volumes, &["app", "data", "world.dat"], WEB)
            .unwrap();
        std::fs::create_dir_all(dir.path().join("volumes/app/data/folder")).unwrap();
        history
            .capture(&volumes, &["app", "data", "folder"], WEB)
            .unwrap();
        assert!(history.read_index().unwrap().is_empty());
    }

    #[test]
    fn old_versions_are_purged_with_their_contents() {
        let (_dir, _volumes, history) = server();
        let long_ago = now_ms() - 31 * DAY_MS;
        history
            .keep("/app/data/a", b"ancient", WEB, long_ago)
            .unwrap();
        let ancient = history.read_index().unwrap()[0].sha256.clone();
        history
            .keep("/app/data/a", b"recent", WEB, now_ms())
            .unwrap();

        let entries = history.read_index().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].size, 6);
        assert!(!history.object(&ancient).exists());
    }

    #[test]
    fn identical_contents_are_stored_once() {
        let (_dir, _volumes, history) = server();
        history.keep("/app/data/a", b"same", WEB, now_ms()).unwrap();
        history.keep("/app/data/b", b"same", WEB, now_ms()).unwrap();
        let objects = std::fs::read_dir(history.dir.join("objects"))
            .unwrap()
            .count();
        assert_eq!(objects, 1);
        assert_eq!(history.read_index().unwrap().len(), 2);
    }

    #[test]
    fn a_zero_retention_keeps_nothing_and_erases_what_was_kept() {
        let (dir, volumes, history) = server();
        put(&dir, "a.yml", b"one");
        history
            .capture(&volumes, &["app", "data", "a.yml"], WEB)
            .unwrap();
        assert_eq!(history.read_index().unwrap().len(), 1);

        let off = history.clone().with_policy(HistoryPolicy {
            days: 0,
            max_mb: 100,
        });
        put(&dir, "a.yml", b"two");
        off.capture(&volumes, &["app", "data", "a.yml"], WEB)
            .unwrap();
        off.purge().unwrap();
        assert!(off.read_index().unwrap().is_empty());
        assert_eq!(
            std::fs::read_dir(off.dir.join("objects")).unwrap().count(),
            0
        );
    }

    #[test]
    fn a_smaller_space_keeps_only_the_newest_versions() {
        let (_dir, _volumes, history) = server();
        let tiny = history.with_policy(HistoryPolicy {
            days: 30,
            max_mb: 0,
        });
        tiny.keep("/app/data/a", b"first", WEB, now_ms()).unwrap();
        // Sans place du tout, même la dernière version part.
        assert!(tiny.read_index().unwrap().is_empty());
    }

    #[test]
    fn deleted_files_of_a_folder_are_listed_with_their_last_version() {
        let (dir, volumes, history) = server();
        put(&dir, "gone.yml", b"v1");
        history
            .capture(&volumes, &["app", "data", "gone.yml"], WEB)
            .unwrap();
        put(&dir, "gone.yml", b"v2");
        history
            .capture(&volumes, &["app", "data", "gone.yml"], WEB)
            .unwrap();
        std::fs::remove_file(dir.path().join("volumes/app/data/gone.yml")).unwrap();
        // Toujours là : pas dans la liste.
        put(&dir, "kept.yml", b"x");
        history
            .capture(&volumes, &["app", "data", "kept.yml"], WEB)
            .unwrap();
        // Dans un sous-dossier : pas un fichier direct.
        std::fs::create_dir_all(dir.path().join("volumes/app/data/sub")).unwrap();
        put(&dir, "sub/deep.yml", b"x");
        history
            .capture(&volumes, &["app", "data", "sub", "deep.yml"], WEB)
            .unwrap();
        std::fs::remove_file(dir.path().join("volumes/app/data/sub/deep.yml")).unwrap();

        let deleted = history.deleted(&volumes, &["app", "data"]).unwrap();
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0].path, "/app/data/gone.yml");
        let content = history
            .read(&["app", "data", "gone.yml"], deleted[0].version.id)
            .unwrap();
        assert_eq!(content, b"v2");
    }
}
