//! Extraction d'une archive (zip, `.tar`, `.tar.gz`, `.tar.xz`) dans un volume (ADR-0017), et
//! restauration d'une sauvegarde (`.tar.zst`, ADR-0018).
//!
//! Chaque entrée passe par l'accès sûr aux volumes ([`Volumes`]) : chemin absolu, `..`, lien
//! symbolique ou matériel et entrée spéciale ne sont jamais écrits. Un lien relatif qui mène à un
//! fichier de la même extraction est remplacé par une copie de ce fichier (bibliothèques
//! `libfoo.so.1 → libfoo.so.1.2.3` d'un serveur livré en `.tar.xz`) ; les autres sont ignorés.
//! Taille totale et nombre d'entrées sont plafonnés (archive piégée).

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use flate2::read::GzDecoder;

use super::InstallError;
use crate::fs::volumes::{Owner, Volumes};

/// Taille totale extraite au plus.
pub(crate) const MAX_EXTRACTED_BYTES: u64 = 8 * 1024 * 1024 * 1024;
/// Entrées au plus.
pub(crate) const MAX_ENTRIES: usize = 100_000;
/// Mémoire du décodeur xz (dictionnaire), en Kio : une archive qui en demande plus est refusée.
const XZ_MEMORY_KIB: u32 = 256 * 1024;
/// Plafonds d'une restauration : l'archive vient du nœud, mais son contenu des conteneurs.
pub(crate) const MAX_RESTORED_BYTES: u64 = 4 * 1024 * 1024 * 1024 * 1024;
pub(crate) const MAX_RESTORED_ENTRIES: usize = 50_000_000;

const FILE_MODE: u32 = 0o644;
const EXECUTABLE_MODE: u32 = 0o755;
/// Liens suivis à la suite au plus (`a → b → c`) avant d'abandonner.
const MAX_LINK_HOPS: usize = 8;

/// Ce qui a été extrait.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Extracted {
    pub files: usize,
    pub bytes: u64,
    /// Liens et entrées spéciales ignorés.
    pub skipped: usize,
}

/// Ce qu'une opération peut encore extraire, toutes archives comprises : une archive imbriquée
/// dans une autre n'ouvre pas un nouveau plafond. Partagé entre les extractions d'une même
/// installation, et ramené au quota disque restant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub bytes: u64,
    pub entries: usize,
}

impl Budget {
    /// Plafonds d'une installation ou d'un contenu.
    pub fn install() -> Self {
        Self {
            bytes: MAX_EXTRACTED_BYTES,
            entries: MAX_ENTRIES,
        }
    }

    fn restore() -> Self {
        Self {
            bytes: MAX_RESTORED_BYTES,
            entries: MAX_RESTORED_ENTRIES,
        }
    }

    /// Ramené à `bytes` au plus (quota disque restant).
    pub fn capped(self, bytes: u64) -> Self {
        Self {
            bytes: self.bytes.min(bytes),
            ..self
        }
    }
}

/// Extrait `from` (chemin du volume) vers `to`. `subdir` : seul ce dossier de l'archive, à la
/// racine de `to`. `executable` : chemins relatifs à `to` rendus exécutables.
pub fn extract(
    volumes: &Volumes,
    from: &[&str],
    to: &[&str],
    subdir: Option<&str>,
    executable: &[String],
    owner: Owner,
    budget: &mut Budget,
) -> Result<Extracted, InstallError> {
    let mut archive = volumes.open(from)?;
    let mut magic = [0_u8; 262];
    let read = read_up_to(&mut archive, &mut magic)?;
    archive.seek(SeekFrom::Start(0)).map_err(io)?;
    volumes.mkdir_all(to, owner)?;
    let prefix: Vec<&str> = subdir.map_or_else(Vec::new, |s| {
        s.split('/').filter(|p| !p.is_empty()).collect()
    });
    let same_owner = |_: &[&str]| Some(owner);
    let mut sink = Sink {
        volumes,
        to,
        prefix: &prefix,
        owner: &same_owner,
        budget,
        extracted: Extracted::default(),
        links: Some(Links::default()),
    };
    let magic = &magic[..read];
    if magic.starts_with(b"PK\x03\x04") || magic.starts_with(b"PK\x05\x06") {
        zip(archive, &mut sink)?;
    } else if magic.starts_with(&[0x1f, 0x8b]) {
        tar(tar::Archive::new(GzDecoder::new(archive)), &mut sink)?;
    } else if magic.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0x00]) {
        let xz = lzma_rust2::XzReader::new_mem_limit(archive, true, XZ_MEMORY_KIB);
        tar(tar::Archive::new(xz), &mut sink)?;
    } else if magic.len() >= 262 && &magic[257..262] == b"ustar" {
        tar(tar::Archive::new(archive), &mut sink)?;
    } else {
        return Err(InstallError::Failed(
            "unsupported archive format".to_owned(),
        ));
    }
    sink.copy_links()?;

    for relative in executable {
        let mut path = to.to_vec();
        path.extend(relative.split('/').filter(|p| !p.is_empty()));
        volumes.set_mode(&path, EXECUTABLE_MODE).map_err(|_| {
            InstallError::Failed(format!("executable `{relative}` not found in the archive"))
        })?;
    }
    Ok(sink.extracted)
}

/// Restaure une sauvegarde (`tar.zst`, entrées `<service>/<volume>/…`) dans les volumes.
/// `owner` donne le propriétaire d'un chemin, ou `None` pour l'ignorer (volume disparu).
pub fn restore(
    volumes: &Volumes,
    archive: impl Read,
    owner: &dyn Fn(&[&str]) -> Option<Owner>,
) -> Result<Extracted, InstallError> {
    let mut budget = Budget::restore();
    let mut sink = Sink {
        volumes,
        to: &[],
        prefix: &[],
        owner,
        budget: &mut budget,
        extracted: Extracted::default(),
        // Une sauvegarde est restaurée telle quelle : ses liens restent ignorés.
        links: None,
    };
    let decoder = zstd::Decoder::new(archive).map_err(io)?;
    tar(tar::Archive::new(decoder), &mut sink)?;
    Ok(sink.extracted)
}

fn read_up_to(file: &mut File, buffer: &mut [u8]) -> Result<usize, InstallError> {
    let mut read = 0;
    while read < buffer.len() {
        match file.read(&mut buffer[read..]).map_err(io)? {
            0 => break,
            n => read += n,
        }
    }
    Ok(read)
}

fn io(error: std::io::Error) -> InstallError {
    InstallError::Failed(format!("archive: {error}"))
}

/// Destination des entrées, avec les plafonds.
struct Sink<'a> {
    volumes: &'a Volumes,
    to: &'a [&'a str],
    prefix: &'a [&'a str],
    /// Propriétaire d'un chemin de destination ; `None` : entrée ignorée.
    owner: &'a dyn Fn(&[&str]) -> Option<Owner>,
    /// Ce que l'opération peut encore extraire, décompté au fil des entrées.
    budget: &'a mut Budget,
    extracted: Extracted,
    /// Liens à remplacer par une copie, à la fin de l'extraction ; `None` : liens ignorés.
    links: Option<Links>,
}

/// Fichiers et liens d'une extraction, chemins complets dans le volume.
#[derive(Default)]
struct Links {
    /// Fichier extrait → exécutable.
    files: HashMap<Vec<String>, bool>,
    /// Lien → chemin visé, déjà résolu sous la destination.
    links: Vec<(Vec<String>, Vec<String>, Owner)>,
}

enum Kind {
    Directory,
    File {
        executable: bool,
    },
    /// Lien symbolique (`target` relatif à son dossier) ou matériel (`target` : entrée de
    /// l'archive) ; jamais créé comme lien.
    Link {
        target: String,
        hard: bool,
    },
    /// Entrée spéciale : jamais écrite.
    Other,
}

impl Sink<'_> {
    /// Chemin de destination, ou `None` pour une entrée hors de `subdir` (ignorée).
    fn target(&self, name: &str) -> Result<Option<Vec<String>>, InstallError> {
        // Archives produites sous Windows : séparateurs `\`.
        let name = name.replace('\\', "/");
        if name.starts_with('/') || name.contains('\0') || name.chars().nth(1) == Some(':') {
            return Err(InstallError::Failed(format!(
                "unsafe archive entry `{name}`"
            )));
        }
        let mut parts = Vec::new();
        for part in name.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    return Err(InstallError::Failed(format!(
                        "unsafe archive entry `{name}`"
                    )));
                }
                part => parts.push(part),
            }
        }
        if !parts.starts_with(self.prefix) {
            return Ok(None);
        }
        let relative = &parts[self.prefix.len()..];
        if relative.is_empty() {
            return Ok(None);
        }
        if !relative
            .iter()
            .all(|p| kuro_protocol::limits::is_valid_name(p))
        {
            return Err(InstallError::Failed(format!(
                "unsafe archive entry `{name}`"
            )));
        }
        let mut path: Vec<String> = self.to.iter().map(|s| (*s).to_owned()).collect();
        path.extend(relative.iter().map(|s| (*s).to_owned()));
        Ok(Some(path))
    }

    fn entry(
        &mut self,
        name: &str,
        kind: Kind,
        mut content: impl Read,
    ) -> Result<(), InstallError> {
        if self.budget.entries == 0 {
            return Err(InstallError::Failed(
                "more entries than this operation may extract".to_owned(),
            ));
        }
        self.budget.entries -= 1;
        let Some(path) = self.target(name)? else {
            return Ok(());
        };
        let path: Vec<&str> = path.iter().map(String::as_str).collect();
        // Service et volume d'une sauvegarde : points de montage, déjà là.
        if matches!(kind, Kind::Directory) && path.len() < 3 {
            return Ok(());
        }
        let Some(owner) = (self.owner)(&path) else {
            self.extracted.skipped += 1;
            return Ok(());
        };
        match kind {
            Kind::Other => self.extracted.skipped += 1,
            Kind::Link { target, hard } => {
                let resolved = if hard {
                    self.target(&target).ok().flatten()
                } else {
                    self.resolve_relative(&path, &target)
                };
                match (&mut self.links, resolved) {
                    (Some(links), Some(resolved)) => {
                        let path = path.iter().map(|s| (*s).to_owned()).collect();
                        links.links.push((path, resolved, owner));
                    }
                    _ => self.extracted.skipped += 1,
                }
            }
            Kind::Directory => self.volumes.mkdir_all(&path, owner)?,
            Kind::File { executable } => {
                self.volumes.mkdir_all(&path[..path.len() - 1], owner)?;
                let mode = if executable {
                    EXECUTABLE_MODE
                } else {
                    FILE_MODE
                };
                let mut file = self.volumes.create(&path, owner, mode)?;
                let remaining = self.budget.bytes;
                let copied = std::io::copy(&mut (&mut content).take(remaining + 1), &mut file.file)
                    .map_err(io)?;
                if copied > remaining {
                    return Err(InstallError::Failed(format!(
                        "archive expands beyond the {remaining} bytes this operation may extract"
                    )));
                }
                file.commit()?;
                self.budget.bytes -= copied;
                self.extracted.bytes += copied;
                self.extracted.files += 1;
                if let Some(links) = &mut self.links {
                    let path = path.iter().map(|s| (*s).to_owned()).collect();
                    links.files.insert(path, executable);
                }
            }
        }
        Ok(())
    }

    /// Cible d'un lien symbolique relatif, sous la destination ; `None` : lien absolu ou qui en
    /// sort, ignoré.
    fn resolve_relative(&self, link: &[&str], target: &str) -> Option<Vec<String>> {
        let target = target.replace('\\', "/");
        if target.starts_with('/') || target.contains('\0') {
            return None;
        }
        let mut path: Vec<String> = link[..link.len() - 1]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        for part in target.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    if path.len() <= self.to.len() {
                        return None;
                    }
                    path.pop();
                }
                part if kuro_protocol::limits::is_valid_name(part) => path.push(part.to_owned()),
                _ => return None,
            }
        }
        (path.len() > self.to.len()).then_some(path)
    }

    /// Remplace chaque lien qui mène à un fichier extrait par une copie de ce fichier.
    fn copy_links(&mut self) -> Result<(), InstallError> {
        let Some(links) = self.links.take() else {
            return Ok(());
        };
        let by_path: HashMap<&Vec<String>, &Vec<String>> = links
            .links
            .iter()
            .map(|(path, target, _)| (path, target))
            .collect();
        for (path, target, owner) in &links.links {
            let mut target = target;
            let mut hops = 0;
            while !links.files.contains_key(target) && hops < MAX_LINK_HOPS {
                match by_path.get(target) {
                    Some(next) => target = next,
                    None => break,
                }
                hops += 1;
            }
            let Some(&executable) = links.files.get(target) else {
                // Dossier, fichier absent de l'archive, boucle : ignoré.
                self.extracted.skipped += 1;
                continue;
            };
            let source: Vec<&str> = target.iter().map(String::as_str).collect();
            let destination: Vec<&str> = path.iter().map(String::as_str).collect();
            let mut source = self.volumes.open(&source)?;
            self.volumes
                .mkdir_all(&destination[..destination.len() - 1], *owner)?;
            let mode = if executable {
                EXECUTABLE_MODE
            } else {
                FILE_MODE
            };
            let mut file = self.volumes.create(&destination, *owner, mode)?;
            let remaining = self.budget.bytes;
            let copied = std::io::copy(&mut (&mut source).take(remaining + 1), &mut file.file)
                .map_err(io)?;
            if copied > remaining {
                return Err(InstallError::Failed(format!(
                    "archive expands beyond the {remaining} bytes this operation may extract"
                )));
            }
            file.commit()?;
            self.budget.bytes -= copied;
            self.extracted.bytes += copied;
            self.extracted.files += 1;
        }
        Ok(())
    }
}

fn zip(archive: File, sink: &mut Sink<'_>) -> Result<(), InstallError> {
    let mut zip =
        ::zip::ZipArchive::new(archive).map_err(|e| InstallError::Failed(format!("zip: {e}")))?;
    for index in 0..zip.len() {
        let entry = zip
            .by_index(index)
            .map_err(|e| InstallError::Failed(format!("zip: {e}")))?;
        let name = entry
            .name()
            .map_err(|e| InstallError::Failed(format!("zip: {e}")))?
            .into_owned();
        let mode = entry.unix_mode();
        let kind = if entry.is_dir() {
            Kind::Directory
        } else if entry.is_symlink() {
            Kind::Other
        } else {
            Kind::File {
                executable: mode.is_some_and(|m| m & 0o111 != 0),
            }
        };
        sink.entry(&name, kind, entry)?;
    }
    Ok(())
}

fn tar<R: Read>(mut archive: tar::Archive<R>, sink: &mut Sink<'_>) -> Result<(), InstallError> {
    for entry in archive.entries().map_err(io)? {
        let entry = entry.map_err(io)?;
        let name = String::from_utf8_lossy(&entry.path_bytes()).into_owned();
        let kind = match entry.header().entry_type() {
            tar::EntryType::Directory => Kind::Directory,
            tar::EntryType::Regular | tar::EntryType::Continuous => Kind::File {
                executable: entry.header().mode().is_ok_and(|m| m & 0o111 != 0),
            },
            kind @ (tar::EntryType::Symlink | tar::EntryType::Link) => {
                match entry.link_name_bytes() {
                    Some(target) => Kind::Link {
                        target: String::from_utf8_lossy(&target).into_owned(),
                        hard: kind == tar::EntryType::Link,
                    },
                    None => Kind::Other,
                }
            }
            _ => Kind::Other,
        };
        sink.entry(&name, kind, entry)?;
    }
    Ok(())
}
