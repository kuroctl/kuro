//! `kuro-prefab-index <dossier des prefabs> <dossier de sortie>` : construit un store de prefabs
//! (ADR-0042) depuis un dossier au format du dépôt `kuroctl/prefabs`. Chaque prefab valide
//! devient une archive `archives/<id>-<version>.kprefab` ; `index.json` les liste avec leur
//! somme SHA-256. La signature de l'index (minisign) est laissée à qui publie.
//!
//! Même chargement que l'import de l'API ; un prefab invalide fait échouer la construction, et
//! deux dossiers au même `id@version` sont refusés.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use kuro_prefab::Package;
use kuro_prefab::assets::load_assets;
use kuro_prefab::files::zip_folder;
use kuro_prefab::package::{MANIFEST_FILE, read_folder};
use kuro_prefab::store::{ARCHIVES_DIR, INDEX_FILE, IndexEntry, StoreIndex};

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let (Some(root), Some(out)) = (
        args.next().map(PathBuf::from),
        args.next().map(PathBuf::from),
    ) else {
        eprintln!("usage: kuro-prefab-index <prefabs directory> <output directory>");
        return ExitCode::from(2);
    };
    match build(&root, &out) {
        Ok(count) => {
            println!("{count} prefab(s) written to {}", out.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("✗ {error}");
            ExitCode::FAILURE
        }
    }
}

fn build(root: &Path, out: &Path) -> Result<usize, String> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
        .map_err(|e| format!("cannot read {}: {e}", root.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.join(MANIFEST_FILE).is_file())
        .collect();
    dirs.sort();
    if dirs.is_empty() {
        return Err(format!("no prefab found under {}", root.display()));
    }
    let archives = out.join(ARCHIVES_DIR);
    std::fs::create_dir_all(&archives).map_err(|e| e.to_string())?;

    let mut seen: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut entries = Vec::new();
    for dir in &dirs {
        let name = dir
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let package = Package::load_dir(dir).map_err(|e| format!("{name}: {e}"))?;
        let assets = load_assets(dir, &package.manifest).map_err(|e| format!("{name}: {e}"))?;
        let label = format!("{}@{}", package.manifest.id, package.manifest.version);
        if let Some(first) = seen.insert(label.clone(), dir.clone()) {
            return Err(format!(
                "{name}: {label} already used by {}",
                first.display()
            ));
        }
        let files = read_folder(dir).map_err(|e| format!("{name}: {e}"))?;
        let bytes = zip_folder(&package.manifest.id, &files).map_err(|e| format!("{name}: {e}"))?;
        let file = format!(
            "{}-{}.kprefab",
            package.manifest.id, package.manifest.version
        );
        std::fs::write(archives.join(&file), &bytes).map_err(|e| format!("{name}: {e}"))?;
        let digest = <sha2::Sha256 as sha2::Digest>::digest(&bytes);
        let sha256 = digest
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        entries.push(IndexEntry::describe(
            &package,
            &assets,
            format!("{ARCHIVES_DIR}/{file}"),
            sha256,
            bytes.len() as u64,
        ));
        println!("✓ {name} ({label})");
    }
    let generated_at = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    let index = StoreIndex::new(generated_at, entries);
    let json = serde_json::to_vec_pretty(&index).map_err(|e| e.to_string())?;
    std::fs::write(out.join(INDEX_FILE), json).map_err(|e| e.to_string())?;
    Ok(index.prefabs.len())
}
