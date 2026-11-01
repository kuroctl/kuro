//! `kuro-api pki …`

use crate::audit::application::record;
use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::config::Config;
use crate::pki::application;
use crate::pki::infra::PkiFiles;
use clap::Subcommand;
use serde_json::json;

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Crée la CA et le certificat serveur s'ils n'existent pas. Ne régénère jamais l'existant.
    Init {
        /// Nom DNS ou adresse IP sous lequel les daemons joignent l'API (répétable).
        #[arg(long = "name", required = true)]
        names: Vec<String>,
    },
}

pub async fn run(command: Command, config: &Config) -> anyhow::Result<()> {
    match command {
        Command::Init { names } => {
            // Base jointe avant de créer quoi que ce soit : une CA créée sans son événement
            // d'audit ne le serait plus jamais (une relance garde l'existant).
            let pool = super::connect(config).await?;
            let files = PkiFiles::from(&config.daemon);
            let report = application::init(&files, &names)?;
            println!("CA          {}", state(report.ca_created, &files.ca_cert));
            println!(
                "server cert {}",
                state(report.server_created, &files.server_cert)
            );
            println!("CA fingerprint: sha256:{}", report.ca_fingerprint);

            if report.ca_created || report.server_created {
                let event = AuditEvent::success(AuditAction::PkiInitialized).details(json!({
                    "ca_created": report.ca_created,
                    "server_certificate_created": report.server_created,
                    "server_names": names,
                    "ca_fingerprint": report.ca_fingerprint,
                }));
                record(&pool, &AuditContext::cli("pki init"), event).await?;
            }
            Ok(())
        }
    }
}

fn state(created: bool, path: &std::path::Path) -> String {
    let verb = if created { "created" } else { "kept   " };
    format!("{verb} {}", path.display())
}
