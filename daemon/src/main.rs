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
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, clap::Subcommand)]
enum Command {
    /// Processus SFTP (ADR-0047), lancé par `kuro-sftp.service` sous son propre utilisateur :
    /// sans socket Docker, il ne parle qu'au daemon.
    Sftp,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = Config::load(&cli.config).context("loading configuration")?;
    kuro_runtime::telemetry::init(&config.log)?;
    if let Some(Command::Sftp) = cli.command {
        return sftp_worker(&config).await;
    }
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
    let (mut requests, endpoint) = start_sftp_bridge(&config, &manager, &shutdown)?;
    let mut servers = Servers { manager, outgoing };

    // Seule une révocation arrête le lien : le service s'arrête en erreur, visible dans systemd.
    link::run_with(
        &config,
        Some(&mut servers),
        requests.as_mut(),
        endpoint,
        &shutdown,
    )
    .await?;
    tracing::info!("daemon stopped");
    Ok(())
}

/// Côté daemon du SFTP (ADR-0047) : clé d'hôte, file d'audit, socket du processus SFTP. Rend la
/// file des requêtes vers l'API et le point d'accès annoncé dans `hello`.
#[cfg(unix)]
fn start_sftp_bridge(
    config: &Config,
    manager: &Manager,
    shutdown: &CancellationToken,
) -> anyhow::Result<(
    Option<link::ApiRequestQueue>,
    Option<kuro_protocol::daemon::SftpEndpoint>,
)> {
    use std::sync::Arc;

    use kuro_daemon::sftp::{bridge, hostkey, spool::Spool};

    let Some(port) = config.sftp.announced_port() else {
        return Ok((None, None));
    };
    let key = hostkey::ensure(&config.sftp.host_key).context("sftp host key")?;
    let spool = Arc::new(Spool::open(&config.sftp.spool).context("sftp audit spool")?);
    let (api, requests) = link::channel();
    tokio::spawn(bridge::forward_audit(
        Arc::clone(&spool),
        api.clone(),
        shutdown.clone(),
    ));
    tokio::spawn({
        let sftp = config.sftp.clone();
        let manager = manager.clone();
        let shutdown = shutdown.clone();
        async move {
            if let Err(error) = bridge::serve(&sftp, Some(manager), api, spool, shutdown).await {
                tracing::error!(%error, "sftp bridge stopped");
            }
        }
    });
    let endpoint = kuro_protocol::daemon::SftpEndpoint {
        port,
        host_key_sha256: hostkey::fingerprint(&key),
    };
    tracing::info!(port, fingerprint = %endpoint.host_key_sha256, "sftp enabled");
    Ok((Some(requests), Some(endpoint)))
}

#[cfg(not(unix))]
fn start_sftp_bridge(
    config: &Config,
    _manager: &Manager,
    _shutdown: &CancellationToken,
) -> anyhow::Result<(
    Option<link::ApiRequestQueue>,
    Option<kuro_protocol::daemon::SftpEndpoint>,
)> {
    if config.sftp.bind.is_some() {
        tracing::warn!("sftp is only served on Linux: sftp.bind ignored");
    }
    Ok((None, None))
}

#[cfg(target_os = "linux")]
async fn sftp_worker(config: &Config) -> anyhow::Result<()> {
    let shutdown = CancellationToken::new();
    tokio::spawn({
        let shutdown = shutdown.clone();
        async move {
            kuro_runtime::shutdown::signal().await;
            shutdown.cancel();
        }
    });
    kuro_daemon::sftp::worker::run(config, shutdown)
        .await
        .context("sftp worker")?;
    tracing::info!("sftp worker stopped");
    Ok(())
}

/// Le confinement des chemins repose sur `openat2` (ADR-0015) : pas de SFTP ailleurs.
#[cfg(not(target_os = "linux"))]
async fn sftp_worker(_config: &Config) -> anyhow::Result<()> {
    anyhow::bail!("the sftp worker only runs on Linux")
}
