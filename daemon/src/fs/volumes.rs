//! Fichiers des volumes d'un serveur (ADR-0015), accessibles pendant que le serveur tourne.
//!
//! Les chemins sont des segments déjà validés ([`kuro_protocol::limits::path_segments`]), relatifs
//! à `volumes/` : `[service, volume, …]`. Aucun lien symbolique n'est jamais suivi.
//!
//! - Linux : chaque accès part d'un descripteur de `volumes/` et passe par `openat2`
//!   (`RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS`) : le noyau garantit qu'un conteneur ne peut pas
//!   substituer un lien entre la vérification et l'accès.
//! - Ailleurs (développement sous Windows ou macOS) : vérification de chaque segment, **non sûre**
//!   face à un conteneur malveillant.

use std::io::Read;
use std::path::{Path, PathBuf};

use kuro_protocol::daemon::{FileContent, FileEntry, FileKind};
use kuro_protocol::limits::MAX_TEXT_FILE_BYTES;

/// Préfixe des fichiers temporaires d'une écriture atomique, masqués dans les listes.
const TEMP_PREFIX: &str = ".kuro-tmp-";

#[derive(Debug, thiserror::Error)]
pub enum VolumeError {
    #[error("path refused")]
    InvalidPath,
    #[error("no such file or directory")]
    NotFound,
    #[error("already exists")]
    AlreadyExists,
    #[error("file changed since it was read")]
    Conflict,
    #[error("not a text file")]
    NotText,
    #[error("file too large")]
    TooLarge,
    #[error("{0}")]
    Io(std::io::Error),
}

impl From<std::io::Error> for VolumeError {
    fn from(error: std::io::Error) -> Self {
        use std::io::ErrorKind;
        match error.kind() {
            ErrorKind::NotFound => Self::NotFound,
            ErrorKind::AlreadyExists | ErrorKind::DirectoryNotEmpty => Self::AlreadyExists,
            ErrorKind::NotADirectory | ErrorKind::IsADirectory | ErrorKind::InvalidInput => {
                Self::InvalidPath
            }
            // ELOOP (lien refusé), EXDEV (sortie de la racine).
            _ if matches!(error.raw_os_error(), Some(40 | 18)) && cfg!(target_os = "linux") => {
                Self::InvalidPath
            }
            _ => Self::Io(error),
        }
    }
}

/// Propriétaire donné aux fichiers et dossiers créés : l'utilisateur du service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Owner {
    pub uid: u32,
    pub gid: u32,
}

/// Volumes d'un serveur : `<dossier du serveur>/volumes`.
#[derive(Debug, Clone)]
pub struct Volumes {
    root: PathBuf,
}

impl Volumes {
    pub fn new(server_root: &Path) -> Self {
        Self {
            root: server_root.join("volumes"),
        }
    }

    pub fn list(&self, path: &[&str]) -> Result<Vec<FileEntry>, VolumeError> {
        let mut entries = sys::list(&self.root, &path.join("/"))?;
        entries.retain(|e| !e.name.starts_with(TEMP_PREFIX));
        entries.sort_by(|a, b| {
            (a.kind != FileKind::Directory, a.name.to_lowercase())
                .cmp(&(b.kind != FileKind::Directory, b.name.to_lowercase()))
        });
        Ok(entries)
    }

    pub fn read(&self, path: &[&str]) -> Result<FileContent, VolumeError> {
        let (bytes, modified_ms) = sys::read(&self.root, &path.join("/"))?;
        text(bytes, modified_ms)
    }

    /// Remplace atomiquement le fichier. `if_sha256` : refus si son contenu a changé.
    pub fn write(
        &self,
        path: &[&str],
        content: &str,
        if_sha256: Option<&str>,
        owner: Owner,
    ) -> Result<(), VolumeError> {
        if content.len() > MAX_TEXT_FILE_BYTES {
            return Err(VolumeError::TooLarge);
        }
        let (parent, name) = split(path)?;
        let temp = format!("{TEMP_PREFIX}{}", uuid::Uuid::now_v7());
        sys::write(
            &self.root,
            &parent,
            name,
            &temp,
            content.as_bytes(),
            if_sha256,
            owner,
        )
    }

    pub fn mkdir(&self, path: &[&str], owner: Owner) -> Result<(), VolumeError> {
        let (parent, name) = split(path)?;
        sys::mkdir(&self.root, &parent, name, owner)
    }

    /// Renomme ou déplace ; refuse d'écraser une entrée existante.
    pub fn rename(&self, from: &[&str], to: &[&str]) -> Result<(), VolumeError> {
        let (from_parent, from_name) = split(from)?;
        let (to_parent, to_name) = split(to)?;
        sys::rename(&self.root, &from_parent, from_name, &to_parent, to_name)
    }

    /// Supprime un fichier, un lien (pas sa cible) ou un dossier et son contenu.
    pub fn delete(&self, path: &[&str]) -> Result<(), VolumeError> {
        let (parent, name) = split(path)?;
        sys::delete(&self.root, &parent, name)
    }

    /// Fichier ordinaire ouvert en lecture (archive à extraire).
    pub fn open(&self, path: &[&str]) -> Result<std::fs::File, VolumeError> {
        split(path)?;
        sys::open(&self.root, &path.join("/"))
    }

    /// Nouveau fichier écrit en flux : il n'apparaît sous son nom qu'à [`NewFile::commit`] ;
    /// abandonné, il disparaît. Un fichier existant est remplacé.
    pub fn create(&self, path: &[&str], owner: Owner, mode: u32) -> Result<NewFile, VolumeError> {
        let (parent, name) = split(path)?;
        let temp = format!("{TEMP_PREFIX}{}", uuid::Uuid::now_v7());
        let (file, pending) = sys::create(&self.root, &parent, name, &temp, owner, mode)?;
        Ok(NewFile { file, pending })
    }

    /// Crée les dossiers manquants du chemin (sous le volume), comme `mkdir -p`.
    pub fn mkdir_all(&self, path: &[&str], owner: Owner) -> Result<(), VolumeError> {
        for end in 3..=path.len() {
            let (parent, name) = split(&path[..end])?;
            sys::ensure_dir(&self.root, &parent, name, owner)?;
        }
        Ok(())
    }

    /// Droits d'un fichier ordinaire (`0o755`…).
    pub fn set_mode(&self, path: &[&str], mode: u32) -> Result<(), VolumeError> {
        let (parent, name) = split(path)?;
        sys::chmod(&self.root, &parent, name, mode)
    }

    /// Vide un volume (`[service, volume]`) sans supprimer son point de montage.
    pub fn clear(&self, volume: [&str; 2]) -> Result<(), VolumeError> {
        for entry in sys::list(&self.root, &volume.join("/"))? {
            let path = [volume[0], volume[1], entry.name.as_str()];
            match self.delete(&path) {
                Ok(()) | Err(VolumeError::NotFound) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

/// Fichier en cours d'écriture (voir [`Volumes::create`]).
pub struct NewFile {
    pub file: std::fs::File,
    pending: sys::Pending,
}

impl NewFile {
    /// Le fichier prend sa place sous son nom définitif.
    pub fn commit(self) -> Result<(), VolumeError> {
        self.file.sync_all()?;
        drop(self.file);
        self.pending.commit()
    }
}

/// Dossier parent et nom. Les deux premiers segments (service, volume) sont des points de
/// montage : on agit toujours en dessous.
fn split<'a>(path: &[&'a str]) -> Result<(String, &'a str), VolumeError> {
    match path {
        [parent @ .., name] if parent.len() >= 2 => Ok((parent.join("/"), name)),
        _ => Err(VolumeError::InvalidPath),
    }
}

fn text(bytes: Vec<u8>, modified_ms: i64) -> Result<FileContent, VolumeError> {
    if bytes.contains(&0) {
        return Err(VolumeError::NotText);
    }
    let sha256 = kuro_runtime::tls::sha256_hex(&bytes);
    let size = bytes.len() as u64;
    let content = String::from_utf8(bytes).map_err(|_| VolumeError::NotText)?;
    Ok(FileContent {
        content,
        size,
        modified_ms,
        sha256,
    })
}

/// Lit au plus [`MAX_TEXT_FILE_BYTES`] ; au-delà, `TooLarge`.
fn read_limited(reader: impl Read) -> Result<Vec<u8>, VolumeError> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_TEXT_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_TEXT_FILE_BYTES {
        return Err(VolumeError::TooLarge);
    }
    Ok(bytes)
}

fn unchanged(current: &[u8], expected: Option<&str>) -> Result<(), VolumeError> {
    match expected {
        Some(expected) if kuro_runtime::tls::sha256_hex(current) != expected => {
            Err(VolumeError::Conflict)
        }
        _ => Ok(()),
    }
}

#[cfg(target_os = "linux")]
mod sys {
    //! `openat2` : résolution confinée par le noyau, sans aucun lien suivi.

    use std::fs::File;
    use std::io::Write;
    use std::os::fd::{AsFd, OwnedFd};
    use std::path::Path;

    use kuro_protocol::daemon::{FileEntry, FileKind};
    use rustix::fs::{
        AtFlags, Dir, FileType, Gid, Mode, OFlags, ResolveFlags, Uid, fchmod, fchown, fstat,
        mkdirat, openat, openat2, renameat, statat, unlinkat,
    };
    use rustix::io::Errno;

    use super::{Owner, VolumeError, read_limited, unchanged};

    const RESOLVE: ResolveFlags = ResolveFlags::BENEATH
        .union(ResolveFlags::NO_SYMLINKS)
        .union(ResolveFlags::NO_MAGICLINKS);
    const FILE_MODE: u32 = 0o644;
    const DIR_MODE: u32 = 0o755;

    fn errno(error: Errno) -> VolumeError {
        VolumeError::from(std::io::Error::from(error))
    }

    /// `volumes/`, contrôlé par le daemon (les conteneurs n'en voient que l'intérieur des volumes).
    fn root(path: &Path) -> Result<OwnedFd, VolumeError> {
        rustix::fs::open(
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(errno)
    }

    fn open_dir(root_path: &Path, relative: &str) -> Result<OwnedFd, VolumeError> {
        let root = root(root_path)?;
        let relative = if relative.is_empty() { "." } else { relative };
        openat2(
            &root,
            relative,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
            RESOLVE,
        )
        .map_err(errno)
    }

    /// Les champs de `Stat` changent de type selon l'architecture.
    fn int<T: TryInto<i64>>(value: T) -> i64 {
        value.try_into().unwrap_or(0)
    }

    fn modified_ms(stat: &rustix::fs::Stat) -> i64 {
        int(stat.st_mtime).saturating_mul(1000) + int(stat.st_mtime_nsec) / 1_000_000
    }

    fn kind(stat: &rustix::fs::Stat) -> FileKind {
        match FileType::from_raw_mode(stat.st_mode) {
            FileType::RegularFile => FileKind::File,
            FileType::Directory => FileKind::Directory,
            FileType::Symlink => FileKind::Symlink,
            _ => FileKind::Other,
        }
    }

    pub(super) fn list(root: &Path, relative: &str) -> Result<Vec<FileEntry>, VolumeError> {
        let dir = open_dir(root, relative)?;
        let mut entries = Vec::new();
        for entry in Dir::read_from(&dir).map_err(errno)? {
            let entry = entry.map_err(errno)?;
            let Ok(name) = entry.file_name().to_str() else {
                // Nom non UTF-8 : inaccessible par l'API, ignoré.
                continue;
            };
            if name == "." || name == ".." {
                continue;
            }
            let Ok(stat) = statat(&dir, name, AtFlags::SYMLINK_NOFOLLOW) else {
                continue;
            };
            entries.push(FileEntry {
                name: name.to_owned(),
                kind: kind(&stat),
                size: u64::try_from(int(stat.st_size)).unwrap_or(0),
                modified_ms: modified_ms(&stat),
            });
        }
        Ok(entries)
    }

    pub(super) fn read(root_path: &Path, relative: &str) -> Result<(Vec<u8>, i64), VolumeError> {
        let root = root(root_path)?;
        let fd = openat2(
            &root,
            relative,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
            RESOLVE,
        )
        .map_err(errno)?;
        let stat = fstat(&fd).map_err(errno)?;
        if kind(&stat) != FileKind::File {
            return Err(VolumeError::InvalidPath);
        }
        let bytes = read_limited(File::from(fd))?;
        Ok((bytes, modified_ms(&stat)))
    }

    /// Contenu actuel de `name` dans `parent`, s'il existe (fichier ordinaire seulement).
    fn current(
        parent: &OwnedFd,
        name: &str,
    ) -> Result<Option<(Vec<u8>, rustix::fs::Stat)>, VolumeError> {
        let stat = match statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => stat,
            Err(Errno::NOENT) => return Ok(None),
            Err(error) => return Err(errno(error)),
        };
        if kind(&stat) != FileKind::File {
            return Err(VolumeError::InvalidPath);
        }
        let fd = openat(
            parent,
            name,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(errno)?;
        Ok(Some((
            read_limited(File::from(fd)).unwrap_or_default(),
            stat,
        )))
    }

    /// Le daemon tourne en root en production ; ailleurs (tests), changer de propriétaire est
    /// refusé et sans conséquence.
    fn give(fd: impl AsFd, uid: u32, gid: u32) -> Result<(), VolumeError> {
        match fchown(fd, Some(Uid::from_raw(uid)), Some(Gid::from_raw(gid))) {
            Ok(()) | Err(Errno::PERM) => Ok(()),
            Err(error) => Err(errno(error)),
        }
    }

    pub(super) fn write(
        root: &Path,
        parent: &str,
        name: &str,
        temp: &str,
        content: &[u8],
        if_sha256: Option<&str>,
        owner: Owner,
    ) -> Result<(), VolumeError> {
        let dir = open_dir(root, parent)?;
        let existing = current(&dir, name)?;
        match (&existing, if_sha256) {
            (Some((bytes, _)), expected) => unchanged(bytes, expected)?,
            // Supprimé depuis sa lecture.
            (None, Some(_)) => return Err(VolumeError::Conflict),
            (None, None) => {}
        }
        let (mode, uid, gid) = match &existing {
            Some((_, stat)) => (stat.st_mode & 0o7777, stat.st_uid, stat.st_gid),
            None => (FILE_MODE, owner.uid, owner.gid),
        };
        let fd = openat(
            &dir,
            temp,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(FILE_MODE),
        )
        .map_err(errno)?;
        let outcome = (|| {
            give(&fd, uid, gid)?;
            fchmod(&fd, Mode::from_raw_mode(mode)).map_err(errno)?;
            let mut file = File::from(fd);
            file.write_all(content)?;
            file.sync_all()?;
            renameat(&dir, temp, &dir, name).map_err(errno)
        })();
        if outcome.is_err() {
            let _ = unlinkat(&dir, temp, AtFlags::empty());
        }
        outcome
    }

    pub(super) fn mkdir(
        root: &Path,
        parent: &str,
        name: &str,
        owner: Owner,
    ) -> Result<(), VolumeError> {
        let dir = open_dir(root, parent)?;
        mkdirat(&dir, name, Mode::from_raw_mode(DIR_MODE)).map_err(errno)?;
        let created = openat(
            &dir,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(errno)?;
        give(&created, owner.uid, owner.gid)
    }

    pub(super) fn rename(
        root: &Path,
        from_parent: &str,
        from: &str,
        to_parent: &str,
        to: &str,
    ) -> Result<(), VolumeError> {
        let source = open_dir(root, from_parent)?;
        let target = open_dir(root, to_parent)?;
        statat(&source, from, AtFlags::SYMLINK_NOFOLLOW).map_err(errno)?;
        match statat(&target, to, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(_) => return Err(VolumeError::AlreadyExists),
            Err(Errno::NOENT) => {}
            Err(error) => return Err(errno(error)),
        }
        renameat(&source, from, &target, to).map_err(|error| match error {
            // Dossier déplacé dans lui-même.
            Errno::INVAL => VolumeError::InvalidPath,
            other => errno(other),
        })
    }

    pub(super) fn delete(root: &Path, parent: &str, name: &str) -> Result<(), VolumeError> {
        let dir = open_dir(root, parent)?;
        remove(&dir, name)
    }

    /// Descend par descripteurs (`O_NOFOLLOW`) : un lien n'est jamais traversé, seulement retiré.
    fn remove(parent: &OwnedFd, name: &str) -> Result<(), VolumeError> {
        let stat = statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(errno)?;
        if kind(&stat) != FileKind::Directory {
            return unlinkat(parent, name, AtFlags::empty()).map_err(errno);
        }
        let dir = openat(
            parent,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(errno)?;
        let mut names = Vec::new();
        for entry in Dir::read_from(&dir).map_err(errno)? {
            let entry = entry.map_err(errno)?;
            let bytes = entry.file_name().to_bytes();
            if bytes != b"." && bytes != b".." {
                names.push(entry.file_name().to_owned());
            }
        }
        for child in names {
            let stat = statat(&dir, child.as_c_str(), AtFlags::SYMLINK_NOFOLLOW).map_err(errno)?;
            if kind(&stat) == FileKind::Directory {
                let child = child
                    .to_str()
                    .map_err(|_| VolumeError::InvalidPath)?
                    .to_owned();
                remove(&dir, &child)?;
            } else {
                unlinkat(&dir, child.as_c_str(), AtFlags::empty()).map_err(errno)?;
            }
        }
        unlinkat(parent, name, AtFlags::REMOVEDIR).map_err(errno)
    }

    pub(super) fn open(root_path: &Path, relative: &str) -> Result<File, VolumeError> {
        let root = root(root_path)?;
        let fd = openat2(
            &root,
            relative,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
            RESOLVE,
        )
        .map_err(errno)?;
        if kind(&fstat(&fd).map_err(errno)?) != FileKind::File {
            return Err(VolumeError::InvalidPath);
        }
        Ok(File::from(fd))
    }

    /// Fichier temporaire du dossier parent, renommé à la validation.
    pub(in crate::fs) struct Pending {
        dir: OwnedFd,
        temp: String,
        name: String,
        committed: bool,
    }

    impl Pending {
        pub(in crate::fs) fn commit(mut self) -> Result<(), VolumeError> {
            if statat(&self.dir, self.name.as_str(), AtFlags::SYMLINK_NOFOLLOW)
                .is_ok_and(|stat| kind(&stat) == FileKind::Directory)
            {
                return Err(VolumeError::AlreadyExists);
            }
            renameat(&self.dir, self.temp.as_str(), &self.dir, self.name.as_str())
                .map_err(errno)?;
            self.committed = true;
            Ok(())
        }
    }

    impl Drop for Pending {
        fn drop(&mut self) {
            if !self.committed {
                let _ = unlinkat(&self.dir, self.temp.as_str(), AtFlags::empty());
            }
        }
    }

    pub(super) fn create(
        root: &Path,
        parent: &str,
        name: &str,
        temp: &str,
        owner: Owner,
        mode: u32,
    ) -> Result<(File, Pending), VolumeError> {
        let dir = open_dir(root, parent)?;
        let fd = openat(
            &dir,
            temp,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(FILE_MODE),
        )
        .map_err(errno)?;
        let pending = Pending {
            dir,
            temp: temp.to_owned(),
            name: name.to_owned(),
            committed: false,
        };
        give(&fd, owner.uid, owner.gid)?;
        fchmod(&fd, Mode::from_raw_mode(mode & 0o7777)).map_err(errno)?;
        Ok((File::from(fd), pending))
    }

    pub(super) fn ensure_dir(
        root: &Path,
        parent: &str,
        name: &str,
        owner: Owner,
    ) -> Result<(), VolumeError> {
        let dir = open_dir(root, parent)?;
        match statat(&dir, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) if kind(&stat) == FileKind::Directory => return Ok(()),
            Ok(_) => return Err(VolumeError::InvalidPath),
            Err(Errno::NOENT) => {}
            Err(error) => return Err(errno(error)),
        }
        mkdirat(&dir, name, Mode::from_raw_mode(DIR_MODE)).map_err(errno)?;
        let created = openat(
            &dir,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(errno)?;
        give(&created, owner.uid, owner.gid)
    }

    pub(super) fn chmod(
        root: &Path,
        parent: &str,
        name: &str,
        mode: u32,
    ) -> Result<(), VolumeError> {
        let dir = open_dir(root, parent)?;
        let fd = openat(
            &dir,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(errno)?;
        if kind(&fstat(&fd).map_err(errno)?) != FileKind::File {
            return Err(VolumeError::InvalidPath);
        }
        fchmod(&fd, Mode::from_raw_mode(mode & 0o7777)).map_err(errno)
    }
}

#[cfg(not(target_os = "linux"))]
mod sys {
    //! Repli de développement : chaque segment est vérifié, mais une course reste possible.

    use std::io::Write;
    use std::path::{Path, PathBuf};

    use kuro_protocol::daemon::{FileEntry, FileKind};

    use super::{Owner, VolumeError, read_limited, unchanged};

    /// Chemin sur l'hôte, aucun segment existant n'étant un lien symbolique.
    fn checked(root: &Path, relative: &str) -> Result<PathBuf, VolumeError> {
        let mut path = root.to_path_buf();
        for segment in relative.split('/').filter(|s| !s.is_empty()) {
            path.push(segment);
            match std::fs::symlink_metadata(&path) {
                Ok(meta) if meta.file_type().is_symlink() => return Err(VolumeError::InvalidPath),
                _ => {}
            }
        }
        Ok(path)
    }

    fn modified_ms(meta: &std::fs::Metadata) -> i64 {
        meta.modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|d| i64::try_from(d.as_millis()).ok())
            .unwrap_or(0)
    }

    fn kind(meta: &std::fs::Metadata) -> FileKind {
        let kind = meta.file_type();
        if kind.is_symlink() {
            FileKind::Symlink
        } else if kind.is_dir() {
            FileKind::Directory
        } else if kind.is_file() {
            FileKind::File
        } else {
            FileKind::Other
        }
    }

    fn directory(root: &Path, relative: &str) -> Result<PathBuf, VolumeError> {
        let path = checked(root, relative)?;
        if !std::fs::symlink_metadata(&path)?.is_dir() {
            return Err(VolumeError::InvalidPath);
        }
        Ok(path)
    }

    pub(super) fn list(root: &Path, relative: &str) -> Result<Vec<FileEntry>, VolumeError> {
        let dir = directory(root, relative)?;
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
                continue;
            };
            entries.push(FileEntry {
                name,
                kind: kind(&meta),
                size: meta.len(),
                modified_ms: modified_ms(&meta),
            });
        }
        Ok(entries)
    }

    pub(super) fn read(root: &Path, relative: &str) -> Result<(Vec<u8>, i64), VolumeError> {
        let path = checked(root, relative)?;
        let meta = std::fs::symlink_metadata(&path)?;
        if !meta.is_file() {
            return Err(VolumeError::InvalidPath);
        }
        let bytes = read_limited(std::fs::File::open(&path)?)?;
        Ok((bytes, modified_ms(&meta)))
    }

    pub(super) fn write(
        root: &Path,
        parent: &str,
        name: &str,
        temp: &str,
        content: &[u8],
        if_sha256: Option<&str>,
        _owner: Owner,
    ) -> Result<(), VolumeError> {
        let dir = directory(root, parent)?;
        let target = dir.join(name);
        match std::fs::symlink_metadata(&target) {
            Ok(meta) if !meta.is_file() => return Err(VolumeError::InvalidPath),
            Ok(_) => unchanged(
                &read_limited(std::fs::File::open(&target)?).unwrap_or_default(),
                if_sha256,
            )?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if if_sha256.is_some() {
                    return Err(VolumeError::Conflict);
                }
            }
            Err(error) => return Err(error.into()),
        }
        let temp = dir.join(temp);
        let outcome = (|| {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            file.write_all(content)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temp, &target)
        })();
        if outcome.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        outcome.map_err(VolumeError::from)
    }

    pub(super) fn mkdir(
        root: &Path,
        parent: &str,
        name: &str,
        _owner: Owner,
    ) -> Result<(), VolumeError> {
        let dir = directory(root, parent)?;
        std::fs::create_dir(dir.join(name))?;
        Ok(())
    }

    pub(super) fn rename(
        root: &Path,
        from_parent: &str,
        from: &str,
        to_parent: &str,
        to: &str,
    ) -> Result<(), VolumeError> {
        let source = directory(root, from_parent)?.join(from);
        let target = directory(root, to_parent)?.join(to);
        std::fs::symlink_metadata(&source)?;
        if std::fs::symlink_metadata(&target).is_ok() {
            return Err(VolumeError::AlreadyExists);
        }
        if target.starts_with(&source) {
            return Err(VolumeError::InvalidPath);
        }
        std::fs::rename(source, target)?;
        Ok(())
    }

    pub(super) fn delete(root: &Path, parent: &str, name: &str) -> Result<(), VolumeError> {
        let path = directory(root, parent)?.join(name);
        let meta = std::fs::symlink_metadata(&path)?;
        // `remove_dir_all` ne suit pas les liens. Un lien est retiré lui-même : comme un fichier,
        // ou comme un dossier sous Windows s'il désigne un dossier.
        if meta.is_dir() {
            std::fs::remove_dir_all(path)?;
        } else if std::fs::remove_file(&path).is_err() && meta.file_type().is_symlink() {
            std::fs::remove_dir(path)?;
        }
        Ok(())
    }

    pub(super) fn open(root: &Path, relative: &str) -> Result<std::fs::File, VolumeError> {
        let path = checked(root, relative)?;
        if !std::fs::symlink_metadata(&path)?.is_file() {
            return Err(VolumeError::InvalidPath);
        }
        Ok(std::fs::File::open(path)?)
    }

    /// Fichier temporaire, renommé à la validation.
    pub(in crate::fs) struct Pending {
        temp: PathBuf,
        target: PathBuf,
        committed: bool,
    }

    impl Pending {
        pub(in crate::fs) fn commit(mut self) -> Result<(), VolumeError> {
            if std::fs::symlink_metadata(&self.target).is_ok_and(|m| m.is_dir()) {
                return Err(VolumeError::AlreadyExists);
            }
            std::fs::rename(&self.temp, &self.target)?;
            self.committed = true;
            Ok(())
        }
    }

    impl Drop for Pending {
        fn drop(&mut self) {
            if !self.committed {
                let _ = std::fs::remove_file(&self.temp);
            }
        }
    }

    pub(super) fn create(
        root: &Path,
        parent: &str,
        name: &str,
        temp: &str,
        _owner: Owner,
        _mode: u32,
    ) -> Result<(std::fs::File, Pending), VolumeError> {
        let dir = directory(root, parent)?;
        let temp = dir.join(temp);
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        Ok((
            file,
            Pending {
                temp,
                target: dir.join(name),
                committed: false,
            },
        ))
    }

    pub(super) fn ensure_dir(
        root: &Path,
        parent: &str,
        name: &str,
        _owner: Owner,
    ) -> Result<(), VolumeError> {
        let path = directory(root, parent)?.join(name);
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() => Ok(()),
            Ok(_) => Err(VolumeError::InvalidPath),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(path)?;
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Sans droits Unix (Windows), seul le fichier est vérifié.
    pub(super) fn chmod(
        root: &Path,
        parent: &str,
        name: &str,
        _mode: u32,
    ) -> Result<(), VolumeError> {
        let path = directory(root, parent)?.join(name);
        if !std::fs::symlink_metadata(&path)?.is_file() {
            return Err(VolumeError::InvalidPath);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(_mode & 0o7777))?;
        }
        Ok(())
    }
}
