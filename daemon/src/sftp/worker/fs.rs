//! Sous-système SFTP d'une session (version 3, ADR-0047), au-dessus de
//! [`crate::fs::volumes::Volumes`] : aucun lien suivi, chemins sous la racine du serveur.
//!
//! Une écriture exige `servers.files.write` (sinon refusée et journalisée une fois), puis les
//! mêmes conditions que le panel, vérifiées par le daemon : aucun déploiement en cours, serveur
//! actif, quota non atteint. Elle est refusée tant que l'audit ne passe plus. Les créations
//! appartiennent à l'utilisateur du service ; chaque écriture est journalisée avec le chemin
//! `/<service>/<volume>/…` que montre l'explorateur du panel.

use std::collections::HashMap;
use std::os::unix::fs::FileExt;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use kuro_protocol::api::{AuthorKind, FileAuthor};
use kuro_protocol::daemon::sftp::{SftpEventKind, SftpPrincipal};
use kuro_protocol::daemon::{FileEntry, FileKind, VersionChannel, VersionReason};
use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Packet, Status, StatusCode, Version,
};
use russh_sftp::server::StatusReply;
use tokio::time::Instant;

use super::client::{Call, DaemonClient};
use super::sessions::Live;
use crate::fs::history::{Capture, History};
use crate::fs::volumes::{Owner, VolumeError, Volumes, WriteMode};
use crate::sftp::ipc::{BridgeMessage, WriteRefusal};
use crate::sftp::vfs::{Location, Tree};

/// Octets rendus au plus par `READ` (un client en demande en général 32 Kio).
const MAX_READ: u32 = 64 * 1024;
/// Entrées par réponse `READDIR` : un paquet reste loin de la limite des clients (256 Kio).
const ENTRIES_PER_READDIR: usize = 100;
/// Descripteurs ouverts au plus par session.
const MAX_HANDLES: usize = 64;
/// Une vérification d'écriture accordée vaut pour les opérations des secondes qui suivent (un
/// envoi de dossier en enchaîne des centaines).
const CHECK_VALIDITY: Duration = Duration::from_secs(2);

const WRITE_PERMISSION: &str = "servers.files.write";
/// Extension d'OpenSSH : renommage qui remplace la cible (WinSCP, `sftp`).
const POSIX_RENAME: &str = "posix-rename@openssh.com";

enum Open {
    Dir(Vec<File>),
    File {
        file: Arc<std::fs::File>,
        /// `[service, volume, …]` : pour l'audit et `FSETSTAT`.
        path: Vec<String>,
        read: bool,
        /// Ouvert en écriture : `file.written` à la fermeture.
        writable: bool,
    },
}

pub struct SftpFs {
    tree: Tree,
    volumes: Volumes,
    /// Historique du serveur (ADR-0049) : la version remplacée est gardée avant chaque écriture.
    history: History,
    live: Arc<Live>,
    client: DaemonClient,
    handles: HashMap<String, Open>,
    next_handle: u64,
    initialized: bool,
    /// Dernière vérification d'écriture accordée par le daemon.
    checked: Option<Instant>,
}

impl SftpFs {
    pub fn new(
        tree: Tree,
        volumes: Volumes,
        history: History,
        live: Arc<Live>,
        client: DaemonClient,
    ) -> Self {
        Self {
            tree,
            volumes,
            history,
            live,
            client,
            handles: HashMap::new(),
            next_handle: 0,
            initialized: false,
            checked: None,
        }
    }

    fn handle(&mut self, open: Open) -> Result<String, StatusReply> {
        if self.handles.len() >= MAX_HANDLES {
            return Err(StatusCode::Failure.with_message("too many open handles"));
        }
        self.next_handle += 1;
        let handle = self.next_handle.to_string();
        self.handles.insert(handle.clone(), open);
        Ok(handle)
    }

    fn locate(&self, path: &str) -> Result<Location, StatusReply> {
        self.tree.locate(path).ok_or_else(|| {
            self.live.refused();
            StatusReply::from(StatusCode::NoSuchFile)
        })
    }

    /// Chemin réel où l'on peut créer, renommer ou supprimer : sous un volume, jamais le service
    /// ni le volume eux-mêmes.
    fn writable_path(&self, path: &str) -> Result<Vec<String>, StatusReply> {
        match self.locate(path)? {
            Location::Real(segments) if segments.len() >= 3 => Ok(segments),
            _ => {
                self.live.refused();
                Err(StatusCode::PermissionDenied
                    .with_message("service and volume folders are fixed"))
            }
        }
    }

    /// Garde la version actuelle du fichier avant de la remplacer. Un échec de l'historique
    /// est journalisé mais n'empêche pas l'écriture.
    async fn keep_version(&self, segments: &[String], reason: VersionReason) {
        let SftpPrincipal::Admin(id) = self.live.info.principal;
        let capture = Capture {
            reason,
            channel: VersionChannel::Sftp,
            author: Some(FileAuthor {
                kind: AuthorKind::Admin,
                id: Some(id),
            }),
        };
        let (history, volumes, path) = (
            self.history.clone(),
            self.volumes.clone(),
            segments.to_vec(),
        );
        let kept = tokio::task::spawn_blocking(move || {
            history.capture(&volumes, &borrowed(&path), capture)
        })
        .await;
        match kept {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::warn!(%error, "file version not kept"),
            Err(error) => tracing::warn!(%error, "file version not kept"),
        }
    }

    fn owner(&self, segments: &[String]) -> Owner {
        let (uid, gid) = segments
            .first()
            .and_then(|service| self.tree.owner(service))
            .unwrap_or((0, 0));
        Owner { uid, gid }
    }

    /// Écriture permise maintenant ? Le droit d'abord (refus journalisé une fois), puis l'audit,
    /// puis les conditions du serveur, demandées au daemon.
    async fn may_write(&mut self) -> Result<(), StatusReply> {
        if !self.live.write.load(Ordering::Relaxed) {
            self.live.forbidden(WRITE_PERMISSION);
            return Err(StatusCode::PermissionDenied.with_message("write access not granted"));
        }
        if self.live.audit_blocked() {
            return Err(StatusCode::Failure.with_message("audit unavailable, writing is paused"));
        }
        if self.checked.is_some_and(|at| at.elapsed() < CHECK_VALIDITY) {
            return Ok(());
        }
        let refusal = match self
            .client
            .call(Call::Check(self.live.info.server_id))
            .await
        {
            Ok(BridgeMessage::Check { refusal, .. }) => refusal,
            _ => Some(WriteRefusal::Unavailable),
        };
        match refusal {
            None => {
                self.checked = Some(Instant::now());
                Ok(())
            }
            Some(refusal) => {
                self.checked = None;
                Err(StatusCode::Failure.with_message(refusal.message()))
            }
        }
    }

    fn error(&self, error: VolumeError) -> StatusReply {
        match error {
            VolumeError::NotFound => StatusCode::NoSuchFile.into(),
            // Lien, fichier spécial, chemin refusé par le noyau.
            VolumeError::InvalidPath => {
                self.live.refused();
                StatusCode::PermissionDenied.into()
            }
            VolumeError::AlreadyExists => {
                StatusCode::Failure.with_message("already exists or not empty")
            }
            VolumeError::Io(error) => {
                tracing::warn!(%error, "sftp file access failed");
                StatusCode::Failure.into()
            }
            _ => StatusCode::Failure.into(),
        }
    }

    async fn stat_of(&self, path: &str) -> Result<FileAttributes, StatusReply> {
        match self.locate(path)? {
            Location::Root | Location::Service(_) => Ok(directory_attributes()),
            Location::Real(segments) => {
                let volumes = self.volumes.clone();
                let entry = blocking(move || volumes.stat(&borrowed(&segments)))
                    .await
                    .map_err(|e| self.error(e))?;
                Ok(attributes(&entry))
            }
        }
    }

    /// `SETSTAT` et `FSETSTAT` : droits (sans setuid, setgid ni sticky), taille, dates. Un
    /// changement de propriétaire est refusé.
    async fn apply_attributes(
        &mut self,
        segments: Vec<String>,
        attrs: FileAttributes,
    ) -> Result<(), StatusReply> {
        if attrs.uid.is_some() || attrs.gid.is_some() {
            self.live.refused();
            return Err(StatusCode::PermissionDenied.with_message("owner changes are not allowed"));
        }
        self.may_write().await?;
        if let Some(size) = attrs.size {
            self.keep_version(&segments, VersionReason::Write).await;
            let (volumes, path) = (self.volumes.clone(), segments.clone());
            blocking(move || volumes.truncate(&borrowed(&path), size))
                .await
                .map_err(|e| self.error(e))?;
            self.live.wrote(SftpEventKind::FileWritten {
                path: display(&segments),
                size,
            });
        }
        if let Some(mode) = attrs.permissions {
            let mode = mode & 0o777;
            let (volumes, path) = (self.volumes.clone(), segments.clone());
            blocking(move || volumes.set_entry_mode(&borrowed(&path), mode))
                .await
                .map_err(|e| self.error(e))?;
            self.live.wrote(SftpEventKind::ModeChanged {
                path: display(&segments),
                mode,
            });
        }
        if let (Some(atime), Some(mtime)) = (attrs.atime, attrs.mtime) {
            let (volumes, path) = (self.volumes.clone(), segments.clone());
            blocking(move || {
                volumes.set_times(&borrowed(&path), i64::from(atime), i64::from(mtime))
            })
            .await
            .map_err(|e| self.error(e))?;
        }
        Ok(())
    }

    async fn rename_to(
        &mut self,
        from: String,
        to: String,
        replace: bool,
    ) -> Result<(), StatusReply> {
        self.may_write().await?;
        let from = self.writable_path(&from)?;
        let to = self.writable_path(&to)?;
        if replace {
            self.keep_version(&to, VersionReason::Write).await;
        }
        let (volumes, source, target) = (self.volumes.clone(), from.clone(), to.clone());
        blocking(move || {
            if replace {
                volumes.rename_over(&borrowed(&source), &borrowed(&target))
            } else {
                volumes.rename(&borrowed(&source), &borrowed(&target))
            }
        })
        .await
        .map_err(|e| self.error(e))?;
        self.live.wrote(SftpEventKind::FileRenamed {
            from: display(&from),
            to: display(&to),
        });
        Ok(())
    }
}

/// `/<service>/<volume>/…`, comme l'explorateur du panel et son audit.
fn display(segments: &[String]) -> String {
    format!("/{}", segments.join("/"))
}

fn borrowed(segments: &[String]) -> Vec<&str> {
    segments.iter().map(String::as_str).collect()
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, VolumeError> + Send + 'static,
) -> Result<T, VolumeError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| VolumeError::Io(std::io::Error::other(e.to_string())))?
}

fn ok(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: "Ok".to_owned(),
        language_tag: "en-US".to_owned(),
    }
}

fn directory_attributes() -> FileAttributes {
    FileAttributes {
        permissions: Some(0o040_755),
        ..FileAttributes::default()
    }
}

fn attributes(entry: &FileEntry) -> FileAttributes {
    let mtime = u32::try_from(entry.modified_ms / 1000).ok();
    FileAttributes {
        size: Some(entry.size),
        permissions: Some(match entry.kind {
            FileKind::Directory => 0o040_755,
            FileKind::File => 0o100_644,
            FileKind::Symlink => 0o120_777,
            FileKind::Other => 0o000_644,
        }),
        atime: mtime,
        mtime,
        ..FileAttributes::default()
    }
}

fn file_attributes(meta: &std::fs::Metadata) -> FileAttributes {
    let modified_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0);
    attributes(&FileEntry {
        name: String::new(),
        kind: FileKind::File,
        size: meta.len(),
        modified_ms,
    })
}

fn write_mode(flags: OpenFlags) -> Option<WriteMode> {
    let write = flags
        .intersects(OpenFlags::WRITE | OpenFlags::APPEND | OpenFlags::CREATE | OpenFlags::TRUNCATE);
    write.then(|| WriteMode {
        read: flags.contains(OpenFlags::READ),
        create: flags.contains(OpenFlags::CREATE),
        truncate: flags.contains(OpenFlags::TRUNCATE),
        exclusive: flags.contains(OpenFlags::EXCLUDE),
        append: flags.contains(OpenFlags::APPEND),
    })
}

/// Deux chaînes du format SSH (longueur sur 4 octets puis contenu) : données de `posix-rename`.
fn two_strings(data: &[u8]) -> Option<(String, String)> {
    let mut rest = data;
    let mut next = || {
        let (length, tail) = rest.split_first_chunk::<4>()?;
        let length = usize::try_from(u32::from_be_bytes(*length)).ok()?;
        if tail.len() < length {
            return None;
        }
        let (value, tail) = tail.split_at(length);
        rest = tail;
        String::from_utf8(value.to_vec()).ok()
    };
    Some((next()?, next()?))
}

impl russh_sftp::server::Handler for SftpFs {
    type Error = StatusReply;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported.into()
    }

    async fn init(
        &mut self,
        _version: u32,
        _extensions: HashMap<String, String>,
    ) -> Result<Version, Self::Error> {
        if self.initialized {
            return Err(StatusCode::BadMessage.into());
        }
        self.initialized = true;
        let mut version = Version::new();
        version
            .extensions
            .insert(POSIX_RENAME.to_owned(), "1".to_owned());
        Ok(version)
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        Ok(Name {
            id,
            files: vec![File::dummy(Tree::canonical(&path))],
        })
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        // Un lien n'est jamais suivi : `stat` décrit le lien lui-même, comme `lstat`.
        let attrs = self.stat_of(&path).await?;
        Ok(Attrs { id, attrs })
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let attrs = self.stat_of(&path).await?;
        Ok(Attrs { id, attrs })
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, Self::Error> {
        let files = match self.locate(&path)? {
            Location::Root => self
                .tree
                .services()
                .map(|service| File::new(service, directory_attributes()))
                .collect(),
            Location::Service(service) => self
                .tree
                .volumes(&service)
                .unwrap_or_default()
                .iter()
                .map(|volume| File::new(volume.as_str(), directory_attributes()))
                .collect(),
            Location::Real(segments) => {
                let volumes = self.volumes.clone();
                blocking(move || volumes.list(&borrowed(&segments)))
                    .await
                    .map_err(|e| self.error(e))?
                    .iter()
                    .map(|entry| File::new(entry.name.as_str(), attributes(entry)))
                    .collect()
            }
        };
        let handle = self.handle(Open::Dir(files))?;
        Ok(Handle { id, handle })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, Self::Error> {
        match self.handles.get_mut(&handle) {
            Some(Open::Dir(files)) if !files.is_empty() => {
                let take = files.len().min(ENTRIES_PER_READDIR);
                Ok(Name {
                    id,
                    files: files.drain(..take).collect(),
                })
            }
            Some(Open::Dir(_)) => Err(StatusCode::Eof.into()),
            _ => Err(StatusCode::Failure.with_message("not a directory handle")),
        }
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        pflags: OpenFlags,
        _attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        let open = match write_mode(pflags) {
            Some(mode) => {
                self.may_write().await?;
                let segments = self.writable_path(&filename)?;
                let owner = self.owner(&segments);
                if !mode.exclusive {
                    self.keep_version(&segments, VersionReason::Write).await;
                }
                let (volumes, path) = (self.volumes.clone(), segments.clone());
                let (file, _created) =
                    blocking(move || volumes.open_for_write(&borrowed(&path), mode, owner))
                        .await
                        .map_err(|e| self.error(e))?;
                Open::File {
                    file: Arc::new(file),
                    path: segments,
                    read: false,
                    writable: true,
                }
            }
            None => {
                let Location::Real(segments) = self.locate(&filename)? else {
                    self.live.refused();
                    return Err(StatusCode::PermissionDenied.into());
                };
                let (volumes, path) = (self.volumes.clone(), segments.clone());
                let file = blocking(move || volumes.open(&borrowed(&path)))
                    .await
                    .map_err(|e| self.error(e))?;
                Open::File {
                    file: Arc::new(file),
                    path: segments,
                    read: false,
                    writable: false,
                }
            }
        };
        let handle = self.handle(open)?;
        Ok(Handle { id, handle })
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        let Some(Open::File { file, read, .. }) = self.handles.get_mut(&handle) else {
            return Err(StatusCode::Failure.with_message("not a file handle"));
        };
        let file = Arc::clone(file);
        let first = !*read;
        *read = true;
        let len = usize::try_from(len.min(MAX_READ)).unwrap_or(0);
        let data = blocking(move || {
            let mut buffer = vec![0u8; len];
            let count = file.read_at(&mut buffer, offset)?;
            buffer.truncate(count);
            Ok(buffer)
        })
        .await
        .map_err(|e| self.error(e))?;
        if data.is_empty() {
            return Err(StatusCode::Eof.into());
        }
        if first {
            self.live.files_read.fetch_add(1, Ordering::Relaxed);
        }
        self.live
            .bytes_read
            .fetch_add(data.len() as u64, Ordering::Relaxed);
        Ok(Data { id, data })
    }

    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, Self::Error> {
        let file = match self.handles.get(&handle) {
            Some(Open::File {
                file,
                writable: true,
                ..
            }) => Arc::clone(file),
            Some(Open::File { .. }) => {
                return Err(StatusCode::PermissionDenied.with_message("opened read-only"));
            }
            _ => return Err(StatusCode::Failure.with_message("not a file handle")),
        };
        // Le droit peut avoir été retiré au dernier bail, l'audit bloqué depuis l'ouverture.
        if !self.live.write.load(Ordering::Relaxed) || self.live.audit_blocked() {
            return Err(StatusCode::PermissionDenied.with_message("writing is no longer allowed"));
        }
        let length = data.len() as u64;
        blocking(move || Ok(file.write_all_at(&data, offset)?))
            .await
            .map_err(|e| self.error(e))?;
        self.live.bytes_written.fetch_add(length, Ordering::Relaxed);
        Ok(ok(id))
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, Self::Error> {
        let attrs = match self.handles.get(&handle) {
            Some(Open::Dir(_)) => directory_attributes(),
            Some(Open::File { file, .. }) => {
                let file = Arc::clone(file);
                let meta = blocking(move || Ok(file.metadata()?))
                    .await
                    .map_err(|e| self.error(e))?;
                file_attributes(&meta)
            }
            None => return Err(StatusCode::Failure.with_message("unknown handle")),
        };
        Ok(Attrs { id, attrs })
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        match self.handles.remove(&handle) {
            Some(Open::File {
                file,
                path,
                writable: true,
                ..
            }) => {
                let size = blocking(move || {
                    file.sync_all()?;
                    Ok(file.metadata()?.len())
                })
                .await
                .map_err(|e| self.error(e))?;
                self.live.wrote(SftpEventKind::FileWritten {
                    path: display(&path),
                    size,
                });
                Ok(ok(id))
            }
            Some(_) => Ok(ok(id)),
            None => Err(StatusCode::Failure.with_message("unknown handle")),
        }
    }

    async fn setstat(
        &mut self,
        id: u32,
        path: String,
        attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        let segments = self.writable_path(&path)?;
        self.apply_attributes(segments, attrs).await?;
        Ok(ok(id))
    }

    async fn fsetstat(
        &mut self,
        id: u32,
        handle: String,
        attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        let Some(Open::File { path, .. }) = self.handles.get(&handle) else {
            return Err(StatusCode::Failure.with_message("not a file handle"));
        };
        let segments = path.clone();
        self.apply_attributes(segments, attrs).await?;
        Ok(ok(id))
    }

    async fn remove(&mut self, id: u32, filename: String) -> Result<Status, Self::Error> {
        self.may_write().await?;
        let segments = self.writable_path(&filename)?;
        self.keep_version(&segments, VersionReason::Delete).await;
        let (volumes, path) = (self.volumes.clone(), segments.clone());
        blocking(move || volumes.remove_entry(&borrowed(&path), false))
            .await
            .map_err(|e| self.error(e))?;
        self.live.wrote(SftpEventKind::FileDeleted {
            path: display(&segments),
        });
        Ok(ok(id))
    }

    async fn mkdir(
        &mut self,
        id: u32,
        path: String,
        _attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        self.may_write().await?;
        let segments = self.writable_path(&path)?;
        let owner = self.owner(&segments);
        let (volumes, target) = (self.volumes.clone(), segments.clone());
        blocking(move || volumes.mkdir(&borrowed(&target), owner))
            .await
            .map_err(|e| self.error(e))?;
        self.live.wrote(SftpEventKind::DirectoryCreated {
            path: display(&segments),
        });
        Ok(ok(id))
    }

    async fn rmdir(&mut self, id: u32, path: String) -> Result<Status, Self::Error> {
        self.may_write().await?;
        let segments = self.writable_path(&path)?;
        let (volumes, target) = (self.volumes.clone(), segments.clone());
        blocking(move || volumes.remove_entry(&borrowed(&target), true))
            .await
            .map_err(|e| self.error(e))?;
        self.live.wrote(SftpEventKind::FileDeleted {
            path: display(&segments),
        });
        Ok(ok(id))
    }

    async fn rename(
        &mut self,
        id: u32,
        oldpath: String,
        newpath: String,
    ) -> Result<Status, Self::Error> {
        self.rename_to(oldpath, newpath, false).await?;
        Ok(ok(id))
    }

    async fn extended(
        &mut self,
        id: u32,
        request: String,
        data: Vec<u8>,
    ) -> Result<Packet, Self::Error> {
        if request != POSIX_RENAME {
            return Err(StatusCode::OpUnsupported.into());
        }
        let (from, to) = two_strings(&data).ok_or(StatusReply::from(StatusCode::BadMessage))?;
        self.rename_to(from, to, true).await?;
        Ok(Packet::Status(ok(id)))
    }

    async fn symlink(
        &mut self,
        _id: u32,
        _linkpath: String,
        _targetpath: String,
    ) -> Result<Status, Self::Error> {
        self.live.refused();
        Err(StatusCode::OpUnsupported.with_message("symbolic links are not supported"))
    }

    async fn readlink(&mut self, _id: u32, _path: String) -> Result<Name, Self::Error> {
        self.live.refused();
        Err(StatusCode::OpUnsupported.with_message("symbolic links are not followed"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_rename_data_is_two_ssh_strings() {
        let mut data = Vec::new();
        for text in ["/a.filepart", "/a"] {
            data.extend_from_slice(&u32::try_from(text.len()).unwrap().to_be_bytes());
            data.extend_from_slice(text.as_bytes());
        }
        assert_eq!(
            two_strings(&data),
            Some(("/a.filepart".to_owned(), "/a".to_owned()))
        );
        assert_eq!(two_strings(&data[..5]), None);
    }

    #[test]
    fn open_flags_map_to_write_modes() {
        assert_eq!(write_mode(OpenFlags::READ), None);
        let mode = write_mode(OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE).unwrap();
        assert!(mode.create && mode.truncate && !mode.exclusive && !mode.read);
    }
}
