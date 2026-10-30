//! Fichiers d'un prefab : un dossier, ou une archive `.kprefab` (zip) lue en mémoire. Le même
//! chargement et la même validation s'appliquent aux deux.
//!
//! Une archive vient d'un import : elle est bornée (taille, nombre de fichiers, taille
//! décompressée, chaque fichier) et ses chemins sont refusés s'ils sortent du prefab ou désignent
//! un lien symbolique.

use std::collections::BTreeMap;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use crate::package::{LoadError, MANIFEST_FILE};

/// Taille maximale d'une archive importée.
pub const MAX_ARCHIVE_BYTES: usize = 10 * 1024 * 1024;
/// Fichiers au plus dans une archive.
pub const MAX_ARCHIVE_ENTRIES: usize = 256;
/// Taille décompressée totale au plus.
pub const MAX_UNPACKED_BYTES: u64 = 32 * 1024 * 1024;
/// Taille d'un fichier de l'archive au plus (la plus grande limite d'un fichier lu).
pub const MAX_ENTRY_BYTES: u64 = 2 * 1024 * 1024;

/// Source des fichiers d'un prefab, adressés par chemin relatif (`templates/Game.ini`).
pub trait PrefabFiles {
    /// Contenu du fichier, au plus `max` octets ; `None` s'il n'existe pas.
    fn read(&self, relative: &str, max: u64) -> Result<Option<Vec<u8>>, LoadError>;
    /// Fichiers directement sous `locales/` (chemins relatifs).
    fn locale_files(&self) -> Result<Vec<String>, LoadError>;
}

/// Prefab dans un dossier (prefabs officiels).
#[derive(Debug, Clone, Copy)]
pub struct DirFiles<'a>(pub &'a Path);

impl PrefabFiles for DirFiles<'_> {
    fn read(&self, relative: &str, max: u64) -> Result<Option<Vec<u8>>, LoadError> {
        let path = self.0.join(relative);
        let io = |source| LoadError::Io {
            path: path.clone(),
            source,
        };
        let size = match std::fs::metadata(&path) {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io(error)),
        };
        if size > max {
            return Err(LoadError::TooLarge { path, max });
        }
        std::fs::read(&path).map(Some).map_err(io)
    }

    fn locale_files(&self) -> Result<Vec<String>, LoadError> {
        let dir = self.0.join("locales");
        if !dir.is_dir() {
            return Ok(Vec::new());
        }
        let entries = std::fs::read_dir(&dir).map_err(|source| LoadError::Io {
            path: dir.clone(),
            source,
        })?;
        let mut files = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| LoadError::Io {
                path: dir.clone(),
                source,
            })?;
            if let Some(name) = entry.file_name().to_str() {
                files.push(format!("locales/{name}"));
            }
        }
        files.sort();
        Ok(files)
    }
}

/// Prefab lu depuis une archive `.kprefab`, entièrement en mémoire.
#[derive(Debug, Clone, Default)]
pub struct ArchiveFiles {
    files: BTreeMap<String, Vec<u8>>,
}

fn invalid(reason: impl Into<String>) -> LoadError {
    LoadError::Archive(reason.into())
}

impl ArchiveFiles {
    /// Lit l'archive. `prefab.yaml` peut être à la racine ou dans un unique dossier de tête
    /// (archive d'un dossier).
    pub fn from_zip(bytes: &[u8]) -> Result<Self, LoadError> {
        if bytes.len() > MAX_ARCHIVE_BYTES {
            return Err(invalid(format!("larger than {MAX_ARCHIVE_BYTES} bytes")));
        }
        let mut archive =
            zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| invalid("not a zip archive"))?;
        if archive.len() > MAX_ARCHIVE_ENTRIES {
            return Err(invalid(format!("more than {MAX_ARCHIVE_ENTRIES} files")));
        }
        let mut files = BTreeMap::new();
        let mut unpacked = 0u64;
        for index in 0..archive.len() {
            let mut entry = archive
                .by_index(index)
                .map_err(|_| invalid("unreadable entry"))?;
            if entry.is_dir() {
                continue;
            }
            let label = entry
                .name()
                .map(|name| name.into_owned())
                .unwrap_or_default();
            if entry.is_symlink() {
                return Err(invalid(format!("symbolic link: {label}")));
            }
            let name = entry
                .enclosed_name()
                .ok_or_else(|| invalid(format!("unsafe path: {label}")))?;
            let name =
                relative_name(&name).ok_or_else(|| invalid(format!("unsafe path: {label}")))?;
            // La taille annoncée peut mentir : la lecture elle-même est bornée.
            let mut data = Vec::new();
            (&mut entry)
                .take(MAX_ENTRY_BYTES + 1)
                .read_to_end(&mut data)
                .map_err(|_| invalid(format!("unreadable file: {name}")))?;
            let size = data.len() as u64;
            if size > MAX_ENTRY_BYTES {
                return Err(LoadError::TooLarge {
                    path: PathBuf::from(&name),
                    max: MAX_ENTRY_BYTES,
                });
            }
            unpacked += size;
            if unpacked > MAX_UNPACKED_BYTES {
                return Err(invalid(format!(
                    "more than {MAX_UNPACKED_BYTES} bytes once unpacked"
                )));
            }
            files.insert(name, data);
        }
        Ok(Self {
            files: strip_single_root(files)?,
        })
    }
}

impl ArchiveFiles {
    /// Fichiers déjà en mémoire (brouillon du Prefab Studio) : mêmes bornes et mêmes chemins
    /// autorisés qu'une archive.
    pub fn from_map(files: BTreeMap<String, Vec<u8>>) -> Result<Self, LoadError> {
        if files.len() > MAX_ARCHIVE_ENTRIES {
            return Err(invalid(format!("more than {MAX_ARCHIVE_ENTRIES} files")));
        }
        let mut checked = BTreeMap::new();
        let mut total = 0u64;
        for (name, data) in files {
            let safe = (!name.contains('\\') && !name.starts_with('/'))
                .then(|| relative_name(Path::new(&name)))
                .flatten()
                .filter(|safe| *safe == name)
                .ok_or_else(|| invalid(format!("unsafe path: {name}")))?;
            let size = data.len() as u64;
            if size > MAX_ENTRY_BYTES {
                return Err(LoadError::TooLarge {
                    path: PathBuf::from(&safe),
                    max: MAX_ENTRY_BYTES,
                });
            }
            total += size;
            if total > MAX_UNPACKED_BYTES {
                return Err(invalid(format!("more than {MAX_UNPACKED_BYTES} bytes")));
            }
            checked.insert(safe, data);
        }
        if !checked.contains_key(MANIFEST_FILE) {
            return Err(invalid(format!("no {MANIFEST_FILE} at the root")));
        }
        Ok(Self { files: checked })
    }
}

/// Chemin relatif en `/`, sans composant vide ni spécial.
fn relative_name(path: &Path) -> Option<String> {
    let parts: Vec<&str> = path
        .components()
        .map(|component| match component {
            std::path::Component::Normal(part) => part.to_str(),
            _ => None,
        })
        .collect::<Option<_>>()?;
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Archive d'un dossier (`minecraft-java/prefab.yaml`) : le dossier de tête est retiré.
fn strip_single_root(
    files: BTreeMap<String, Vec<u8>>,
) -> Result<BTreeMap<String, Vec<u8>>, LoadError> {
    if files.contains_key(MANIFEST_FILE) {
        return Ok(files);
    }
    let roots: std::collections::BTreeSet<&str> = files
        .keys()
        .filter_map(|name| name.split_once('/').map(|(root, _)| root))
        .collect();
    let root = match roots.iter().next() {
        Some(root) if roots.len() == 1 && files.keys().all(|name| name.contains('/')) => {
            (*root).to_owned()
        }
        _ => return Err(invalid(format!("no {MANIFEST_FILE} at the root"))),
    };
    let prefix = format!("{root}/");
    let stripped: BTreeMap<String, Vec<u8>> = files
        .into_iter()
        .filter_map(|(name, data)| {
            name.strip_prefix(&prefix)
                .map(|rest| (rest.to_owned(), data))
        })
        .collect();
    if stripped.contains_key(MANIFEST_FILE) {
        Ok(stripped)
    } else {
        Err(invalid(format!("no {MANIFEST_FILE} at the root")))
    }
}

impl PrefabFiles for ArchiveFiles {
    fn read(&self, relative: &str, max: u64) -> Result<Option<Vec<u8>>, LoadError> {
        match self.files.get(relative) {
            None => Ok(None),
            Some(data) if data.len() as u64 > max => Err(LoadError::TooLarge {
                path: PathBuf::from(relative),
                max,
            }),
            Some(data) => Ok(Some(data.clone())),
        }
    }

    fn locale_files(&self) -> Result<Vec<String>, LoadError> {
        Ok(self
            .files
            .keys()
            .filter(|name| {
                name.strip_prefix("locales/")
                    .is_some_and(|rest| !rest.contains('/'))
            })
            .cloned()
            .collect())
    }
}

/// Archive zip des fichiers d'un prefab, sous un dossier de tête `<folder>/` (que l'import
/// retire) : une archive `.kprefab` prête à être téléchargée ou publiée dans un store.
pub fn zip_folder(folder: &str, files: &BTreeMap<String, Vec<u8>>) -> std::io::Result<Vec<u8>> {
    use std::io::Write;
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (path, data) in files {
        writer.start_file(format!("{folder}/{path}"), options)?;
        writer.write_all(data)?;
    }
    Ok(writer.finish()?.into_inner())
}
