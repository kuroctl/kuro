//! CLI de maintenance du daemon, destinée à l'exploitation.
//!
//! Un module par groupe de commandes. Les commandes qui interrogent le service en
//! cours d'exécution passeront par un socket d'administration local (à venir).

mod config;
mod enroll;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use kuro_daemon::config::DEFAULT_PATH;

/// Maintenance du daemon Kuro.
#[derive(Debug, Parser)]
#[command(version)]
struct Cli {
    /// Fichier de configuration du daemon.
    #[arg(long, global = true, env = "KURO_DAEMON_CONFIG", default_value = DEFAULT_PATH)]
    config: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Opérations sur la configuration.
    #[command(subcommand)]
    Config(config::Command),
    /// Enrôle cet hôte auprès de l'API et écrit la configuration du daemon.
    Enroll(enroll::EnrollArgs),
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Config(command) => config::run(command, &cli.config),
        Command::Enroll(args) => enroll::run(args, &cli.config),
    }
}
