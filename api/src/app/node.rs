//! `kuro-api node …` : mêmes cas d'usage que `POST /api/admin/v1/nodes…`.

use crate::audit::domain::AuditContext;
use crate::config::Config;
use crate::enrollment::application::{NodeTicket, add_node, reissue_token};
use crate::nodes::application::{PlacementInput, inventory, lifecycle, regions};
use crate::nodes::domain::NoLiveLinks;
use crate::pki::infra::PkiFiles;
use anyhow::{Context, bail};
use clap::{Args, Subcommand};

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Crée un nœud `pending` et affiche son token d'enrôlement (usage unique).
    Add {
        /// Identifiant technique (`[a-z0-9-]`, 63 caractères au plus), non modifiable.
        #[arg(long)]
        name: String,
        #[command(flatten)]
        placement: PlacementArgs,
    },
    /// Réémet un token pour un nœud en attente ou révoqué (les précédents sont invalidés).
    Token {
        #[arg(long)]
        name: String,
    },
    /// Révoque le certificat du nœud. Sa session est coupée au prochain heartbeat.
    Revoke {
        #[arg(long)]
        name: String,
        /// Motif, conservé dans le journal d'audit.
        #[arg(long)]
        reason: Option<String>,
    },
    /// Supprime un nœud en attente, hors ligne ou révoqué (irréversible) ; un nœud en ligne est
    /// refusé, et des serveurs encore assignés aussi. Son certificat reste refusé.
    Delete {
        #[arg(long)]
        name: String,
        /// Confirme la suppression.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Args)]
pub struct PlacementArgs {
    /// Slug de la région (`kuro-api region list`).
    #[arg(long)]
    region: Option<String>,
    /// Libellé affiché, modifiable.
    #[arg(long)]
    display_name: Option<String>,
    /// Localisation libre, par exemple « Paris – OVH GRA ».
    #[arg(long)]
    location: Option<String>,
    #[arg(long, requires = "longitude", allow_negative_numbers = true)]
    latitude: Option<f64>,
    #[arg(long, requires = "latitude", allow_negative_numbers = true)]
    longitude: Option<f64>,
}

pub async fn run(command: Command, config: &Config) -> anyhow::Result<()> {
    let pool = super::connect(config).await?;
    let ttl = config.enrollment.token_ttl();
    // Cette commande ne tient aucune session : l'API la coupe au prochain heartbeat.
    let links = NoLiveLinks;

    match command {
        Command::Add { name, placement } => {
            let region_id = match &placement.region {
                Some(slug) => Some(regions::id_by_slug(&pool, slug).await?),
                None => None,
            };
            let input = PlacementInput {
                region_id,
                display_name: placement.display_name,
                location: placement.location,
                coordinates: placement.latitude.zip(placement.longitude),
            };
            let ticket = add_node(&pool, &AuditContext::cli("node add"), &name, input, ttl).await?;
            println!(
                "node {} added ({}), waiting for enrollment",
                ticket.node.name, ticket.node.id
            );
            print_ticket(&ticket, config)?;
        }
        Command::Token { name } => {
            let id = inventory::id_by_name(&pool, &name).await?;
            let ticket = reissue_token(&pool, &AuditContext::cli("node token"), id, ttl).await?;
            println!(
                "new enrollment token for node {} ({})",
                ticket.node.name, ticket.node.id
            );
            print_ticket(&ticket, config)?;
        }
        Command::Revoke { name, reason } => {
            let id = inventory::id_by_name(&pool, &name).await?;
            lifecycle::revoke(&pool, &AuditContext::cli("node revoke"), &links, id, reason).await?;
            println!("node {name} revoked; issue a new token (`node token`) to re-enroll it");
        }
        Command::Delete { name, yes } => {
            if !yes {
                bail!("deleting node {name} is irreversible: add --yes to confirm");
            }
            let id = inventory::id_by_name(&pool, &name).await?;
            lifecycle::delete(&pool, &AuditContext::cli("node delete"), &links, id).await?;
            println!(
                "node {name} deleted; its certificate stays refused, re-enroll the host as a new node"
            );
        }
    }
    Ok(())
}

fn print_ticket(ticket: &NodeTicket, config: &Config) -> anyhow::Result<()> {
    let ca_fingerprint = PkiFiles::from(&config.daemon)
        .load_ca()
        .context("loading CA")?
        .fingerprint();
    println!();
    println!(
        "enrollment token, valid {} min, shown only once:",
        ticket.expires_in.as_secs() / 60
    );
    println!("  {}", ticket.token.expose());
    println!();
    println!("on the host to enroll, paste the token when prompted (not echoed):");
    println!("  read -rs KURO_ENROLL_TOKEN && export KURO_ENROLL_TOKEN");
    println!(
        "  kuro-daemonctl enroll --api https://<api-host>:{} --ca-fingerprint sha256:{ca_fingerprint}",
        config.daemon.bind.port(),
    );
    Ok(())
}
