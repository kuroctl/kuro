//! `kuro-api audit …` : intégrité et conservation du journal (ADR-0008).

use crate::audit::application;
use crate::audit::domain::AuditContext;
use crate::config::Config;
use anyhow::bail;
use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Recalcule la chaîne de hash ; code de sortie non nul en cas d'altération.
    Verify,
    /// Supprime les événements plus anciens que la durée de conservation des paramètres du
    /// projet. L'API le fait déjà chaque jour.
    Purge,
}

pub async fn run(command: Command, config: &Config) -> anyhow::Result<()> {
    let pool = super::connect(config).await?;
    match command {
        Command::Verify => {
            let report = application::verify(&pool).await?;
            match (report.first_seq, report.last_seq) {
                (Some(first), Some(last)) => {
                    println!("{} events checked (#{first} to #{last})", report.checked);
                }
                _ => println!("audit log is empty"),
            }
            if let Some(seq) = report.broken_at {
                bail!("audit chain broken at event #{seq}: the log has been altered");
            }
            println!("audit chain intact");
        }
        Command::Purge => {
            let context = AuditContext::cli("audit purge");
            let (retention, deleted) = application::purge_expired(&pool, &context).await?;
            println!("{deleted} events older than {retention} days deleted");
        }
    }
    Ok(())
}
