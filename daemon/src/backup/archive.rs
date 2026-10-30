//! Écriture d'une archive de sauvegarde : `tar` compressé en zstd, lu à travers l'accès sûr aux
//! volumes (ADR-0015). Dossiers et fichiers ordinaires seulement ; un lien n'est jamais suivi.

use std::fs::File;
use std::io::{self, Read, Write};

use kuro_protocol::daemon::FileKind;
use sha2::{Digest, Sha256};

use super::pattern::Ignore;
use super::{BackupError, Meta};
use crate::fs::volumes::{VolumeError, Volumes};

/// Niveau zstd : bon compromis vitesse / taille pour des mondes de jeu.
const LEVEL: i32 = 3;
const DIR_MODE: u32 = 0o755;

/// Ce qui a été archivé.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Archived {
    pub files: usize,
    pub bytes: u64,
}

/// Archive les volumes (`[service, volume]`) dans `out`, moins `ignore`. Retourne les
/// métadonnées de l'archive écrite.
pub fn create(
    volumes: &Volumes,
    roots: &[[String; 2]],
    ignore: &Ignore,
    out: File,
) -> Result<(Meta, Archived), BackupError> {
    let counted = Counted::new(out);
    let encoder = zstd::Encoder::new(counted, LEVEL)?;
    let mut builder = tar::Builder::new(encoder);
    let mut archived = Archived::default();
    for [service, volume] in roots {
        let root = vec![service.clone(), volume.clone()];
        if ignore.matches(&borrowed(&root)) {
            continue;
        }
        directory(&mut builder, &root, 0)?;
        walk(volumes, ignore, &mut builder, &root, &mut archived)?;
    }
    let encoder = builder.into_inner()?;
    let mut counted = encoder.finish()?;
    counted.flush()?;
    counted.inner.sync_all()?;
    let meta = Meta {
        size: counted.bytes,
        sha256: hex(&counted.hasher.finalize()),
    };
    Ok((meta, archived))
}

fn walk<W: Write>(
    volumes: &Volumes,
    ignore: &Ignore,
    builder: &mut tar::Builder<W>,
    dir: &[String],
    archived: &mut Archived,
) -> Result<(), BackupError> {
    let entries = match volumes.list(&borrowed(dir)) {
        Ok(entries) => entries,
        // Supprimé ou remplacé par un lien pendant l'archivage.
        Err(VolumeError::NotFound | VolumeError::InvalidPath) => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let mut path = dir.to_vec();
        path.push(entry.name.clone());
        if ignore.matches(&borrowed(&path)) {
            continue;
        }
        let mtime = u64::try_from(entry.modified_ms / 1000).unwrap_or(0);
        match entry.kind {
            FileKind::Directory => {
                directory(builder, &path, mtime)?;
                walk(volumes, ignore, builder, &path, archived)?;
            }
            FileKind::File => {
                if let Some(bytes) = file(volumes, builder, &path, mtime)? {
                    archived.files += 1;
                    archived.bytes += bytes;
                }
            }
            // Liens et entrées spéciales : jamais archivés.
            FileKind::Symlink | FileKind::Other => {}
        }
    }
    Ok(())
}

fn directory<W: Write>(
    builder: &mut tar::Builder<W>,
    path: &[String],
    mtime: u64,
) -> Result<(), BackupError> {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Directory);
    header.set_size(0);
    header.set_mode(DIR_MODE);
    header.set_mtime(mtime);
    builder.append_data(&mut header, format!("{}/", path.join("/")), io::empty())?;
    Ok(())
}

/// Un fichier qui change pendant la lecture est archivé à la taille relevée à l'ouverture :
/// complété de zéros ou tronqué, l'archive reste lisible.
fn file<W: Write>(
    volumes: &Volumes,
    builder: &mut tar::Builder<W>,
    path: &[String],
    mtime: u64,
) -> Result<Option<u64>, BackupError> {
    let file = match volumes.open(&borrowed(path)) {
        Ok(file) => file,
        Err(VolumeError::NotFound | VolumeError::InvalidPath) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    let size = metadata.len();
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_size(size);
    header.set_mode(mode(&metadata));
    header.set_mtime(mtime);
    let content = file.take(size).chain(io::repeat(0)).take(size);
    builder.append_data(&mut header, path.join("/"), content)?;
    Ok(Some(size))
}

#[cfg(unix)]
fn mode(metadata: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o777
}

#[cfg(not(unix))]
fn mode(_: &std::fs::Metadata) -> u32 {
    0o644
}

fn borrowed(path: &[String]) -> Vec<&str> {
    path.iter().map(String::as_str).collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Compte et hache ce qui part sur le disque.
struct Counted {
    inner: File,
    hasher: Sha256,
    bytes: u64,
}

impl Counted {
    fn new(inner: File) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
            bytes: 0,
        }
    }
}

impl Write for Counted {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.hasher.update(&buf[..written]);
        self.bytes += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
