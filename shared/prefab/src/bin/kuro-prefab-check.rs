//! `kuro-prefab-check <dossier>` : vérifie un dossier de prefabs comme l'import de l'API
//! (ADR-0041). Chaque sous-dossier qui contient un `prefab.yaml` est chargé et validé (manifest,
//! traductions, gabarits, images déclarées) ; deux dossiers au même `id@version` sont refusés,
//! puisqu'ils entreraient en conflit au rechargement du catalogue. Sortie non nulle au premier
//! problème trouvé, après avoir tout listé.
//!
//! Utilisé par la CI de Kuro (sous-module `prefabs/`) et par celle du dépôt `kuroctl/prefabs`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use kuro_prefab::Package;
use kuro_prefab::assets::load_assets;
use kuro_prefab::package::MANIFEST_FILE;

fn main() -> ExitCode {
    let Some(root) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: kuro-prefab-check <prefabs directory>");
        return ExitCode::from(2);
    };
    let dirs = match prefab_dirs(&root) {
        Ok(dirs) => dirs,
        Err(error) => {
            eprintln!("cannot read {}: {error}", root.display());
            return ExitCode::from(2);
        }
    };
    let mut failed = false;
    let mut seen: BTreeMap<String, PathBuf> = BTreeMap::new();
    for dir in &dirs {
        let name = dir
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        match check(dir) {
            Ok(label) => {
                if let Some(first) = seen.insert(label.clone(), dir.clone()) {
                    eprintln!("✗ {name}: {label} already used by {}", first.display());
                    failed = true;
                } else {
                    println!("✓ {name} ({label})");
                }
            }
            Err(error) => {
                eprintln!("✗ {name}: {error}");
                failed = true;
            }
        }
    }
    if dirs.is_empty() {
        eprintln!("no prefab found under {}", root.display());
        return ExitCode::FAILURE;
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Sous-dossiers qui contiennent un `prefab.yaml`, triés.
fn prefab_dirs(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.join(MANIFEST_FILE).is_file())
        .collect();
    dirs.sort();
    Ok(dirs)
}

/// Charge le prefab comme l'import (validation comprise) et ses images ; rend `id@version`.
fn check(dir: &Path) -> Result<String, kuro_prefab::LoadError> {
    let package = Package::load_dir(dir)?;
    load_assets(dir, &package.manifest)?;
    Ok(format!(
        "{}@{}",
        package.manifest.id, package.manifest.version
    ))
}
