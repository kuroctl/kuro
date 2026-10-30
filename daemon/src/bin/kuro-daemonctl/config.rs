//! `kuro-daemonctl config …`

use std::path::Path;

use anyhow::Context;
use clap::Subcommand;
use kuro_daemon::config::Config;

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Charge et valide la configuration, sans démarrer le service.
    Check,
}

pub fn run(command: Command, path: &Path) -> anyhow::Result<()> {
    match command {
        Command::Check => check(path),
    }
}

fn check(path: &Path) -> anyhow::Result<()> {
    let config =
        Config::load(path).with_context(|| format!("invalid configuration {}", path.display()))?;
    println!("configuration OK: {}", path.display());
    println!("  api.url     = {}", config.api.url);
    println!("  tls.ca_cert = {}", config.tls.ca_cert.display());
    println!("  tls.cert    = {}", config.tls.cert.display());
    println!("  tls.key     = {}", config.tls.key.display());
    Ok(())
}
