//! Rechargement à chaud de `cargo xtask dev` : une modification des sources Rust recompile l'API
//! et le daemon, puis relance celui ou ceux qui ont changé. L'API applique ses nouvelles
//! migrations à son démarrage. Le panel a déjà le sien (Vite).
//!
//! Sous Windows, un exécutable lancé ne peut pas être remplacé mais peut être renommé : les
//! binaires en cours sont écartés (`.old`) le temps de la compilation. Si elle échoue, ils
//! reprennent leur nom et la version qui tourne reste en place.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tokio::process::Command;

use crate::workspace::{self, binary, root};

/// Intervalle entre deux examens des sources.
const POLL: Duration = Duration::from_millis(800);

type Fingerprint = Vec<(PathBuf, u64, Option<SystemTime>)>;

/// Ce qu'une modification impose de relancer.
#[derive(Debug, Clone, Copy, Default)]
pub struct Changes {
    pub api: bool,
    pub daemon: bool,
}

/// Sources surveillées, par process qu'elles concernent.
pub struct SourceWatch {
    api: Fingerprint,
    daemon: Fingerprint,
    shared: Fingerprint,
}

fn api_sources() -> Vec<PathBuf> {
    let mut sources = ["api/src", "api/migrations", "api/Cargo.toml", ".sqlx"]
        .map(|path| root().join(path))
        .to_vec();
    // Sources d'une extension compilée avec Kuro (`--watch`).
    sources.extend(super::target::get().watch.iter().cloned());
    sources
}

fn daemon_sources() -> Vec<PathBuf> {
    ["daemon/src", "daemon/Cargo.toml"]
        .map(|path| root().join(path))
        .to_vec()
}

fn shared_sources() -> Vec<PathBuf> {
    ["shared", "Cargo.toml", "Cargo.lock"]
        .map(|path| root().join(path))
        .to_vec()
}

/// Chemins, tailles et dates des fichiers ; les dossiers de compilation sont ignorés.
fn fingerprint(paths: &[PathBuf]) -> Fingerprint {
    let mut files = Vec::new();
    let mut stack: Vec<PathBuf> = paths.to_vec();
    while let Some(path) = stack.pop() {
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            if path.file_name().is_some_and(|name| name == "target") {
                continue;
            }
            if let Ok(entries) = std::fs::read_dir(&path) {
                stack.extend(entries.filter_map(Result::ok).map(|entry| entry.path()));
            }
        } else {
            files.push((path, meta.len(), meta.modified().ok()));
        }
    }
    files.sort();
    files
}

impl SourceWatch {
    pub fn new() -> Self {
        Self {
            api: fingerprint(&api_sources()),
            daemon: fingerprint(&daemon_sources()),
            shared: fingerprint(&shared_sources()),
        }
    }

    /// Attend une modification, une fois les fichiers stables (un éditeur écrit parfois en
    /// plusieurs fois, et un `cargo fmt` touche plusieurs fichiers).
    pub async fn changed(&mut self) -> Changes {
        loop {
            tokio::time::sleep(POLL).await;
            let (api, daemon, shared) = scan();
            if api == self.api && daemon == self.daemon && shared == self.shared {
                continue;
            }
            // Stable : deux examens identiques de suite.
            let mut current = (api, daemon, shared);
            loop {
                tokio::time::sleep(POLL).await;
                let next = scan();
                if next == current {
                    break;
                }
                current = next;
            }
            let (api, daemon, shared) = current;
            let shared_changed = shared != self.shared;
            let changes = Changes {
                api: api != self.api || shared_changed,
                daemon: daemon != self.daemon || shared_changed,
            };
            self.api = api;
            self.daemon = daemon;
            self.shared = shared;
            return changes;
        }
    }
}

fn scan() -> (Fingerprint, Fingerprint, Fingerprint) {
    (
        fingerprint(&api_sources()),
        fingerprint(&daemon_sources()),
        fingerprint(&shared_sources()),
    )
}

/// Binaire écarté le temps d'une compilation.
struct Parked {
    original: PathBuf,
    aside: PathBuf,
}

fn park(path: &Path) -> Option<Parked> {
    if !path.exists() {
        return None;
    }
    let aside = path.with_extension("old");
    let _ = std::fs::remove_file(&aside);
    std::fs::rename(path, &aside).ok().map(|()| Parked {
        original: path.to_path_buf(),
        aside,
    })
}

/// Compile l'API et le daemon : un seul `cargo build` dans le workspace de Kuro, ou deux quand
/// l'API vient du workspace d'une extension.
pub async fn build() -> bool {
    let api = super::target::get();
    let cargo = |packages: &[&str]| {
        let mut build = Command::new("cargo");
        build.args(["build", "--quiet"]);
        for package in packages {
            build.args(["-p", package]);
        }
        build.arg("--bins");
        build
    };
    if api.in_kuro() {
        return workspace::run(cargo(&[&api.bin, "kuro-daemon"]), "cargo build")
            .await
            .is_ok();
    }
    workspace::run(cargo(&["kuro-daemon"]), "cargo build (daemon)")
        .await
        .is_ok()
        && workspace::run_in(
            extension_build(cargo(&[&api.bin])),
            &api.workspace,
            "cargo build (api)",
        )
        .await
        .is_ok()
}

/// Hors du workspace de Kuro, sa configuration Cargo ne s'applique pas : les requêtes de
/// `kuro-api` se vérifient quand même contre son cache `.sqlx/` (comme `.cargo/config.toml` de
/// Kuro), sauf choix contraire de l'appelant.
fn extension_build(mut build: Command) -> Command {
    if std::env::var_os("SQLX_OFFLINE").is_none() {
        build.env("SQLX_OFFLINE", "true");
    }
    build
}

/// Recompile l'API et le daemon. `false` si la compilation échoue : les binaires en cours sont
/// alors remis en place.
pub async fn rebuild() -> bool {
    let parked: Vec<Parked> = [super::target::get().binary(), binary("kuro-daemon")]
        .iter()
        .filter_map(|path| park(path))
        .collect();
    let built = build().await;
    for binary in parked {
        if built || !binary.original.exists() {
            if !built {
                let _ = std::fs::rename(&binary.aside, &binary.original);
            } else {
                // Encore ouvert par le process en cours sous Windows : effacé au prochain tour.
                let _ = std::fs::remove_file(&binary.aside);
            }
        }
    }
    built
}
