use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use kuro_daemon::config::{self, Config};
use kuro_daemon::docker::Docker;
use kuro_daemon::link;
use kuro_daemon::server::{Manager, Servers};
use tokio_util::sync::CancellationToken;

/// Service Kuro installé sur l'hôte.
#[derive(Debug, Parser)]
#[command(version)]
struct Cli {
    /// Fichier de configuration TOML.
    #[arg(long, env = "KURO_DAEMON_CONFIG", default_value = config::DEFAULT_PATH)]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = Config::load(&cli.config).context("loading configuration")?;
    kuro_runtime::telemetry::init(&config.log)?;
    // Le confinement des fichiers des volumes n'est garanti que sous Linux (ADR-0015).
    #[cfg(not(target_os = "linux"))]
    if !config.servers.allow_insecure_fs {
        anyhow::bail!(
            "volume files are only confined on Linux; set servers.allow_insecure_fs = true for a development node"
        );
    }
    #[cfg(not(target_os = "linux"))]
    tracing::warn!("volume files are not confined on this platform (development only)");

    let shutdown = CancellationToken::new();
    tokio::spawn({
        let shutdown = shutdown.clone();
        async move {
            kuro_runtime::shutdown::signal().await;
            shutdown.cancel();
        }
    });

    tracing::info!(api = %config.api.url, "daemon started");

    let docker = Docker::connect(&config.servers.helper_image).context("connecting to docker")?;
    if let Err(error) = docker.ping().await {
        // Le lien reste ouvert : les commandes échoueront en `failed` jusqu'au retour de Docker.
        tracing::warn!(%error, "docker is not reachable");
    }
    let (manager, outgoing) = Manager::with_options(
        docker,
        config.servers.data_root.clone(),
        Default::default(),
        config.web.clone(),
    );
    // Temporaires d'opérations interrompues par l'arrêt précédent.
    manager.clean_up_at_boot().await;
    // Le proxy des sites web reprend les routes des serveurs déployés (ADR-0038).
    manager.restore_proxy().await;
    tokio::spawn({
        let manager = manager.clone();
        async move {
            manager
                .watch_certificates(std::time::Duration::from_secs(30))
                .await
        }
    });
    tokio::spawn({
        let manager = manager.clone();
        async move { manager.watch_events().await }
    });
    tokio::spawn({
        let manager = manager.clone();
        let every = config.servers.disk_measure_interval();
        async move { manager.measure_disks(every).await }
    });
    // Historique des stats : une mesure toutes les 10 s, une minute agrégée envoyée (ADR-0043).
    tokio::spawn({
        let manager = manager.clone();
        async move { manager.collect_metrics().await }
    });
    let mut servers = Servers { manager, outgoing };

    // Seule une révocation arrête le lien : le service s'arrête en erreur, visible dans systemd.
    link::run(&config, Some(&mut servers), &shutdown).await?;
    tracing::info!("daemon stopped");
    Ok(())
}
