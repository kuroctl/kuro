//! API lancée par `cargo xtask dev` : `kuro-api` par défaut, ou le binaire d'un dépôt qui compile
//! Kuro avec des extensions (ADR-0051), par exemple `kuro-cloud` depuis le dépôt de Kuro Cloud,
//! où Kuro est un sous-module.
//!
//! Fixée une fois au lancement (`set`), lue ensuite par l'initialisation, le rechargement et le
//! lancement des process.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, bail};
use tokio::process::Command;

use crate::workspace::root;

#[derive(Debug, Clone)]
pub struct ApiTarget {
    /// Workspace Cargo du binaire de l'API (celui de Kuro par défaut).
    pub workspace: PathBuf,
    /// Binaire, et paquet du même nom (`kuro-api`, `kuro-cloud`…).
    pub bin: String,
    /// URL de la base, si elle remplace celle de la configuration (`--database`).
    pub database_url: Option<String>,
    /// Sources qui relancent l'API en plus de celles de Kuro (une extension).
    pub watch: Vec<PathBuf>,
    /// Extensions du panel admin, `<nom>=<dossier absolu>` (`KURO_PANEL_EXTENSIONS`).
    pub panel_extensions: Vec<String>,
}

static TARGET: OnceLock<ApiTarget> = OnceLock::new();

/// Fixe l'API de ce lancement ; un second appel est une erreur de programmation ignorée.
pub fn set(target: ApiTarget) {
    let _ = TARGET.set(target);
}

/// L'API de ce lancement, `kuro-api` du workspace de Kuro si rien n'a été fixé.
pub fn get() -> &'static ApiTarget {
    TARGET.get_or_init(ApiTarget::kuro)
}

impl ApiTarget {
    pub fn kuro() -> Self {
        Self {
            workspace: root(),
            bin: "kuro-api".to_owned(),
            database_url: None,
            watch: Vec::new(),
            panel_extensions: Vec::new(),
        }
    }

    /// Le binaire est-il compilé dans le workspace de Kuro (cas par défaut) ?
    pub fn in_kuro(&self) -> bool {
        same_dir(&self.workspace, &root())
    }

    /// Binaire compilé (`target/debug/<bin>` de son workspace, ou `CARGO_TARGET_DIR`).
    pub fn binary(&self) -> PathBuf {
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.workspace.join("target"));
        target
            .join("debug")
            .join(format!("{}{}", self.bin, std::env::consts::EXE_SUFFIX))
    }

    /// Commande du binaire de l'API, avec la base choisie. Lancée depuis la racine de Kuro :
    /// les chemins relatifs de sa configuration de dev (PKI, prefabs) y restent valables.
    pub fn command(&self) -> Command {
        let mut command = Command::new(self.binary());
        command.current_dir(root());
        if let Some(url) = &self.database_url {
            command.env("KURO_API_DATABASE__URL", url);
        }
        command
    }

    /// `KURO_PANEL_EXTENSIONS` du panel, s'il y a des extensions.
    pub fn panel_env(&self) -> Option<String> {
        (!self.panel_extensions.is_empty()).then(|| self.panel_extensions.join(","))
    }
}

/// Chemin absolu sans le préfixe `\\?\` de Windows (que Node et Vite lisent mal).
pub fn absolute(path: &Path) -> anyhow::Result<PathBuf> {
    std::path::absolute(path).with_context(|| format!("resolving {}", path.display()))
}

/// `<nom>=<dossier>` d'une extension du panel, dossier rendu absolu.
pub fn panel_extension(entry: &str) -> anyhow::Result<String> {
    let Some((name, dir)) = entry.split_once('=') else {
        bail!("--panel-extension {entry:?}: expected <name>=<dir>");
    };
    let dir = absolute(Path::new(dir))?;
    if !dir.join("extension.ts").is_file() {
        bail!(
            "--panel-extension {name}: {} has no extension.ts",
            dir.display()
        );
    }
    Ok(format!("{name}={}", dir.display()))
}

/// URL de `url` sur la base `name` (le dernier segment du chemin est remplacé).
pub fn with_database(url: &str, name: &str) -> anyhow::Result<String> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        bail!("--database {name:?}: lowercase letters, digits and `_` only");
    }
    let (base, query) = match url.split_once('?') {
        Some((base, query)) => (base, Some(query)),
        None => (url, None),
    };
    let Some((server, _)) = base.rsplit_once('/') else {
        bail!("database url has no database name");
    };
    Ok(match query {
        Some(query) => format!("{server}/{name}?{query}"),
        None => format!("{server}/{name}"),
    })
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_only_the_database_name() {
        assert_eq!(
            with_database("postgres://panel:panel@127.0.0.1:5432/panel", "kuro_cloud").unwrap(),
            "postgres://panel:panel@127.0.0.1:5432/kuro_cloud"
        );
        assert_eq!(
            with_database("postgres://u@h/db?sslmode=disable", "other").unwrap(),
            "postgres://u@h/other?sslmode=disable"
        );
    }

    #[test]
    fn refuses_a_database_name_that_is_not_an_identifier() {
        for name in ["", "Kuro", "a-b", "a;drop", "a b"] {
            assert!(with_database("postgres://u@h/db", name).is_err(), "{name}");
        }
    }
}
