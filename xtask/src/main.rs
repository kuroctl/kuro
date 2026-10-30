//! `cargo xtask` : outillage de développement du workspace.
//!
//! - `cargo xtask dev` : environnement de dev complet (Postgres, API, daemon, panel) en une
//!   commande, initialisé automatiquement au premier lancement.
//! - `cargo xtask prefab-media` : images des prefabs aux dimensions convenues, PNG optimisés sans
//!   perte (`--check` pour la CI).

mod dev;
mod media;
mod workspace;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "cargo xtask", about = "Outillage de développement Kuro")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Lance Postgres, l'API, un daemon enrôlé et le panel ; Ctrl+C arrête tout.
    Dev(dev::DevArgs),
    /// Recadre, réduit et optimise les images déclarées des prefabs (icône, bannière, vignette).
    PrefabMedia(media::MediaArgs),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Dev(args) => dev::run(args).await,
        Command::PrefabMedia(args) => media::run(&args),
    }
}
