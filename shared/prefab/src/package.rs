//! Prefab chargé : manifest, traductions et gabarits.
//!
//! Seuls les fichiers que le manifest déclare (gabarits) et `locales/*.json` sont lus : un
//! prefab ne fait pas lire un chemin arbitraire. Tailles bornées, le contenu peut venir d'un
//! import.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::files::{ArchiveFiles, DirFiles, PrefabFiles};
use crate::manifest::{FileMode, Manifest, Step};
use crate::validate::{self, Issue};

pub const MANIFEST_FILE: &str = "prefab.yaml";
const MAX_MANIFEST_BYTES: u64 = 256 * 1024;
const MAX_LOCALE_BYTES: u64 = 256 * 1024;
const MAX_TEMPLATE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("cannot read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} exceeds {max} bytes")]
    TooLarge { path: PathBuf, max: u64 },
    #[error("invalid prefab.yaml: {0}")]
    Manifest(String),
    #[error("invalid locale {path}: {source}")]
    Locale {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid asset {path}: {reason}")]
    Asset { path: PathBuf, reason: &'static str },
    #[error("invalid prefab:\n{}", format_issues(.0))]
    Invalid(Vec<Issue>),
    /// Archive `.kprefab` refusée (pas un zip, trop grosse, chemin dangereux…).
    #[error("invalid archive: {0}")]
    Archive(String),
}

fn format_issues(issues: &[Issue]) -> String {
    issues
        .iter()
        .map(|issue| format!("  {issue}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Prefab validé, prêt à être stocké ou résolu.
#[derive(Debug, Clone, PartialEq)]
pub struct Package {
    pub manifest: Manifest,
    /// Langue (`fr`, `en`…) → traductions.
    pub locales: BTreeMap<String, Value>,
    /// Chemin relatif (`templates/Game.ini`) → contenu.
    pub templates: BTreeMap<String, String>,
}

impl Package {
    /// Charge et valide le prefab d'un dossier.
    pub fn load_dir(dir: &Path) -> Result<Self, LoadError> {
        Self::load(&DirFiles(dir))
    }

    /// Charge et valide une archive `.kprefab` (zip) ; ses fichiers sont rendus pour les images.
    pub fn load_zip(bytes: &[u8]) -> Result<(Self, ArchiveFiles), LoadError> {
        let files = ArchiveFiles::from_zip(bytes)?;
        Ok((Self::load(&files)?, files))
    }

    /// Charge et valide un prefab, quelle que soit la source de ses fichiers.
    pub fn load(files: &impl PrefabFiles) -> Result<Self, LoadError> {
        let manifest_text = read_text(files, MANIFEST_FILE, MAX_MANIFEST_BYTES)?;
        let manifest = parse_manifest(&manifest_text)?;

        let mut locales = BTreeMap::new();
        for relative in files.locale_files()? {
            let path = PathBuf::from(&relative);
            let Some(lang) = locale_name(&path) else {
                continue;
            };
            let text = read_text(files, &relative, MAX_LOCALE_BYTES)?;
            let value =
                serde_json::from_str(&text).map_err(|source| LoadError::Locale { path, source })?;
            locales.insert(lang, value);
        }

        let mut templates = BTreeMap::new();
        for relative in template_paths(&manifest) {
            // Les chemins ont été vérifiés (relatifs, sans `..`) avant d'être lus.
            if !validate::is_prefab_path(&relative) {
                continue;
            }
            let content = read_text(files, &relative, MAX_TEMPLATE_BYTES)?;
            templates.insert(relative, content);
        }

        let package = Self {
            manifest,
            locales,
            templates,
        };
        let issues = validate::validate(&package);
        if issues.is_empty() {
            Ok(package)
        } else {
            Err(LoadError::Invalid(issues))
        }
    }
}

pub fn parse_manifest(text: &str) -> Result<Manifest, LoadError> {
    serde_saphyr::from_str(text).map_err(|error| LoadError::Manifest(error.to_string()))
}

/// Gabarits référencés par le manifest.
pub fn template_paths(manifest: &Manifest) -> Vec<String> {
    let mut paths: Vec<String> = manifest
        .files
        .values()
        .filter(|file| matches!(file.mode, FileMode::Template | FileMode::User))
        .filter_map(|file| file.template.clone())
        .collect();
    if let Some(install) = &manifest.install {
        paths.extend(install.steps.iter().filter_map(|step| match step {
            Step::Copy { template, .. } => Some(template.clone()),
            _ => None,
        }));
    }
    paths.sort();
    paths.dedup();
    paths
}

fn locale_name(path: &Path) -> Option<String> {
    if path.extension()? != "json" {
        return None;
    }
    let stem = path.file_stem()?.to_str()?;
    let valid = (2..=8).contains(&stem.len())
        && stem.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    valid.then(|| stem.to_owned())
}

/// Fichier texte obligatoire : absent ou non UTF-8, c'est une erreur.
fn read_text(files: &impl PrefabFiles, relative: &str, max: u64) -> Result<String, LoadError> {
    let io = |kind: std::io::ErrorKind| LoadError::Io {
        path: PathBuf::from(relative),
        source: std::io::Error::from(kind),
    };
    let data = files
        .read(relative, max)?
        .ok_or_else(|| io(std::io::ErrorKind::NotFound))?;
    String::from_utf8(data).map_err(|_| io(std::io::ErrorKind::InvalidData))
}

/// Tous les fichiers d'un dossier de prefab que l'import lit : le manifest, les traductions, les
/// gabarits et les images qu'il déclare (chemin relatif → octets). Rien d'autre : un fichier
/// que le manifest ne mentionne pas n'existe pas pour Kuro.
pub fn read_folder(dir: &Path) -> Result<BTreeMap<String, Vec<u8>>, LoadError> {
    let files = DirFiles(dir);
    let manifest_text = read_text(&files, MANIFEST_FILE, MAX_MANIFEST_BYTES)?;
    let manifest = parse_manifest(&manifest_text)?;
    let mut out = BTreeMap::from([(MANIFEST_FILE.to_owned(), manifest_text.into_bytes())]);
    for relative in files.locale_files()? {
        if let Some(data) = files.read(&relative, MAX_LOCALE_BYTES)? {
            out.insert(relative, data);
        }
    }
    for relative in template_paths(&manifest) {
        if validate::is_prefab_path(&relative)
            && let Some(data) = files.read(&relative, MAX_TEMPLATE_BYTES)?
        {
            out.insert(relative, data);
        }
    }
    for relative in [
        manifest.assets.icon.as_deref(),
        manifest.assets.cover.as_deref(),
        manifest.assets.thumbnail.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if validate::is_prefab_path(relative)
            && let Some(data) = files.read(relative, crate::assets::MAX_ASSET_BYTES)?
        {
            out.insert(relative.to_owned(), data);
        }
    }
    Ok(out)
}
