//! Programme de l'orchestrateur : service (`serve`) et commandes d'administration.
//!
//! Partagé par le binaire `kuro-api` (sans extension) et par les binaires qui compilent Kuro
//! avec des extensions (ADR-0051) : tous ont la même CLI et le même démarrage. C'est la
//! coquille du programme : seul module de la bibliothèque autorisé à utiliser `anyhow`.
//!
//! Un module par groupe de commandes. Chaque commande se contente d'appeler un cas d'usage
//! de la bibliothèque, le même que celui exposé en HTTP, et chaque écriture est auditée.

mod admin;
mod audit;
mod node;
mod pki;
mod region;
mod serve;
mod setup;

use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand};
use sqlx::PgPool;

use crate::config::Config;
use crate::extension::ExtensionFactory;
use crate::{db, http};

/// Orchestrateur Kuro.
#[derive(Debug, Parser)]
#[command(version)]
struct Cli {
    /// Fichier de configuration TOML (surchargé par les variables `KURO_API_*`).
    #[arg(long, global = true, env = "KURO_API_CONFIG")]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Démarre l'API REST et le listener des daemons (commande par défaut).
    Serve,
    /// Première mise en route sans panel : super admin et nom du projet (ADR-0045).
    Setup(setup::Command),
    /// Écrit le document OpenAPI (JSON) sur la sortie standard.
    Openapi,
    /// CA privée et certificat serveur du listener daemon.
    #[command(subcommand)]
    Pki(pki::Command),
    /// Gestion des nœuds.
    #[command(subcommand)]
    Node(node::Command),
    /// Régions (emplacement physique des nœuds).
    #[command(subcommand)]
    Region(region::Command),
    /// Comptes admin : premier accès et récupération.
    #[command(subcommand)]
    Admin(admin::Command),
    /// Journal d'audit : vérification d'intégrité, purge de rétention.
    #[command(subcommand)]
    Audit(audit::Command),
}

/// Point d'entrée d'un binaire : lit la ligne de commande et lance la commande demandée.
/// `extensions` : les extensions compilées avec ce binaire (aucune pour `kuro-api`).
pub async fn main(extensions: Vec<ExtensionFactory>) -> anyhow::Result<()> {
    // Avant toute commande : le document OpenAPI publie aussi les permissions des extensions.
    crate::extension::register(&extensions).context("registering extensions")?;
    let cli = Cli::parse();
    let command = cli.command.unwrap_or(Command::Serve);
    if let Command::Openapi = command {
        println!("{}", http::openapi::to_json()?);
        return Ok(());
    }

    let config = Config::load(cli.config.as_deref()).context("loading configuration")?;
    match command {
        Command::Serve => serve::run(config, extensions).await,
        Command::Setup(command) => setup::run(command, &config).await,
        Command::Pki(command) => pki::run(command, &config).await,
        Command::Node(command) => node::run(command, &config).await,
        Command::Region(command) => region::run(command, &config).await,
        Command::Admin(command) => admin::run(command, &config).await,
        Command::Audit(command) => audit::run(command, &config).await,
        Command::Openapi => Ok(()),
    }
}

/// Base prête pour une commande d'administration (migrations appliquées).
async fn connect(config: &Config) -> anyhow::Result<PgPool> {
    let pool = db::connect(&config.database)
        .await
        .context("connecting to database")?;
    db::migrate(&pool).await.context("running migrations")?;
    Ok(pool)
}
