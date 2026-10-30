//! Chemins du workspace et lancement des outils externes (cargo, docker, corepack, node).

use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, bail};
use tokio::process::Command;

/// Racine du workspace : le dossier parent de `xtask/`.
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Binaire compilé du workspace (`target/debug/<name>`), en respectant `CARGO_TARGET_DIR`.
pub fn binary(name: &str) -> PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root().join("target"));
    target
        .join("debug")
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

/// Outil Node installé en script `.cmd` sous Windows (`corepack`, `pnpm`…).
pub fn node_tool(name: &str) -> Command {
    if cfg!(windows) {
        Command::new(format!("{name}.cmd"))
    } else {
        Command::new(name)
    }
}

/// Lance une commande jusqu'au bout, sortie affichée telle quelle ; échoue si elle échoue.
pub async fn run(mut command: Command, what: &str) -> anyhow::Result<()> {
    let status = command
        .current_dir(root())
        .stdin(Stdio::null())
        .status()
        .await
        .with_context(|| format!("{what}: failed to start"))?;
    if !status.success() {
        bail!("{what}: exited with {status}");
    }
    Ok(())
}
