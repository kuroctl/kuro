//! `cargo xtask prefab-media` : met les images des prefabs aux dimensions convenues.
//!
//! Pour chaque `prefabs/<id>/prefab.yaml`, les images déclarées (`assets.icon`, `assets.cover`,
//! `assets.thumbnail`) sont recadrées au centre au bon rapport, puis réduites aux dimensions de la
//! convention, jamais agrandies. Le format est conservé : un PNG est ensuite optimisé sans perte
//! (oxipng), un JPEG réencodé seulement s'il a fallu le redimensionner. Une image déjà conforme et
//! déjà optimisée n'est pas réécrite : la commande peut tourner à chaque ajout.
//!
//! `--check` ne modifie rien et échoue si une image serait changée (utilisable en CI).

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use clap::Args;
use kuro_prefab::assets::AssetKind;
use kuro_prefab::media::{self, Optimized, target};
use kuro_prefab::package::{MANIFEST_FILE, parse_manifest};

use crate::workspace;

#[derive(Debug, Args)]
pub struct MediaArgs {
    /// Ne modifie rien : liste ce qui changerait et échoue s'il y a du travail.
    #[arg(long)]
    check: bool,
    /// Prefabs à traiter (identifiants) ; tous par défaut.
    ids: Vec<String>,
}

pub fn run(args: &MediaArgs) -> anyhow::Result<()> {
    let root = workspace::root().join("prefabs");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .with_context(|| format!("reading {}", root.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.join(MANIFEST_FILE).is_file())
        .filter(|path| {
            args.ids.is_empty()
                || path
                    .file_name()
                    .is_some_and(|name| args.ids.iter().any(|id| name == id.as_str()))
        })
        .collect();
    dirs.sort();

    let mut pending = 0usize;
    for dir in dirs {
        let text = std::fs::read_to_string(dir.join(MANIFEST_FILE))?;
        let manifest =
            parse_manifest(&text).map_err(|error| anyhow::anyhow!("{}: {error}", dir.display()))?;
        for kind in AssetKind::ALL {
            let declared = match kind {
                AssetKind::Icon => manifest.assets.icon.as_deref(),
                AssetKind::Cover => manifest.assets.cover.as_deref(),
                AssetKind::Thumbnail => manifest.assets.thumbnail.as_deref(),
            };
            let Some(relative) = declared else { continue };
            let path = dir.join(relative);
            if !path.is_file() {
                println!(
                    "  absente   {} (déclarée, ignorée à l'import)",
                    path.display()
                );
                continue;
            }
            let original = std::fs::read(&path)?;
            let outcome =
                media::optimize(&original, kind).with_context(|| path.display().to_string())?;
            match outcome {
                None => println!("  conforme  {}", path.display()),
                Some(outcome) => {
                    pending += 1;
                    if !args.check {
                        std::fs::write(&path, &outcome.data)?;
                    }
                    report(&path, &outcome, kind, args.check);
                }
            }
        }
    }
    if args.check && pending > 0 {
        bail!("{pending} image(s) à mettre aux dimensions : lancez `cargo xtask prefab-media`");
    }
    Ok(())
}

fn report(path: &Path, outcome: &Optimized, kind: AssetKind, check: bool) {
    let (before, after) = (outcome.before, outcome.after);
    let verb = if check { "à refaire" } else { "refaite " };
    println!(
        "  {verb} {}: {}×{} {} → {}×{} {}",
        path.display(),
        before.width,
        before.height,
        kib(before.bytes),
        after.width,
        after.height,
        kib(after.bytes)
    );
    if outcome.small {
        let (tw, th) = target(kind);
        println!(
            "            plus petite que {tw}×{th} : jamais agrandie, fournir une image plus grande"
        );
    }
}

fn kib(bytes: u64) -> String {
    format!("{:.0} Kio", bytes as f64 / 1024.0)
}
