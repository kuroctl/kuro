//! `kuro-api region …` : mêmes cas d'usage que `/api/admin/v1/regions`.

use crate::audit::domain::AuditContext;
use crate::config::Config;
use crate::nodes::application::{NewRegion, regions};
use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Crée une région (emplacement physique de nœuds).
    Add {
        /// Identifiant fixe, `[a-z0-9-]`, par exemple `eu-west`.
        #[arg(long)]
        slug: String,
        /// Libellé affiché, par exemple « Europe Ouest ».
        #[arg(long)]
        name: String,
        /// Centre de la région sur la carte.
        #[arg(long, allow_negative_numbers = true)]
        latitude: f64,
        #[arg(long, allow_negative_numbers = true)]
        longitude: f64,
    },
    /// Liste les régions et leurs nœuds.
    List,
}

pub async fn run(command: Command, config: &Config) -> anyhow::Result<()> {
    let pool = super::connect(config).await?;
    match command {
        Command::Add {
            slug,
            name,
            latitude,
            longitude,
        } => {
            let region = regions::create(
                &pool,
                &AuditContext::cli("region add"),
                NewRegion {
                    slug,
                    name,
                    latitude,
                    longitude,
                },
            )
            .await?;
            println!("region {} created ({})", region.slug, region.id);
        }
        Command::List => {
            for region in regions::list(&pool).await? {
                println!(
                    "{:<16} {:<24} {:>3} nodes ({} online, {} offline, {} pending)",
                    region.slug,
                    region.name,
                    region.nodes_total,
                    region.nodes_online,
                    region.nodes_offline,
                    region.nodes_pending,
                );
            }
        }
    }
    Ok(())
}
