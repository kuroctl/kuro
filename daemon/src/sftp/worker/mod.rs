//! Processus SFTP (`kuro-daemon sftp`, ADR-0047) : serveur SSH qui ne sert que le sous-système
//! `sftp`, lancé par `kuro-sftp.service` sous un utilisateur dédié, sans accès à Docker. Il ne
//! parle qu'au daemon, par le socket Unix.

mod client;
mod fs;
mod sessions;
mod ssh;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use kuro_protocol::daemon::sftp::CloseReason;
use russh::server::Config as SshConfig;
use russh::{MethodKind, MethodSet, SshId};
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

pub use client::DaemonClient;
pub use sessions::Sessions;

use crate::config::Config;
use crate::sftp::hostkey;

/// Connexions servies en même temps ; les suivantes attendent leur tour.
const MAX_CONNECTIONS: usize = 256;
/// Fichier de la clé d'hôte transmise par systemd (`LoadCredential=host-key:…`).
const CREDENTIAL: &str = "host-key";

#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    #[error("sftp.bind is not set: SFTP is disabled on this node")]
    Disabled,
    #[error(transparent)]
    HostKey(#[from] hostkey::HostKeyError),
    #[error("failed to listen on {addr}: {source}")]
    Bind {
        addr: std::net::SocketAddr,
        #[source]
        source: std::io::Error,
    },
}

/// Clé d'hôte : celle que systemd transmet, sinon le chemin configuré (développement, tests).
fn host_key_path(config: &Config) -> PathBuf {
    match std::env::var_os("CREDENTIALS_DIRECTORY") {
        Some(dir) => PathBuf::from(dir).join(CREDENTIAL),
        None => config.sftp.host_key.clone(),
    }
}

pub fn ssh_config(host_key: russh::keys::PrivateKey) -> SshConfig {
    SshConfig {
        server_id: SshId::Standard(std::borrow::Cow::Borrowed("SSH-2.0-Kuro")),
        methods: MethodSet::from(&[MethodKind::PublicKey][..]),
        auth_rejection_time: Duration::from_secs(1),
        // Sonde `none` d'OpenSSH : réponse immédiate.
        auth_rejection_time_initial: Some(Duration::ZERO),
        keys: vec![host_key],
        max_auth_attempts: 6,
        inactivity_timeout: Some(Duration::from_secs(15 * 60)),
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 4,
        nodelay: true,
        ..SshConfig::default()
    }
}

/// Sert le SFTP jusqu'à l'annulation de `shutdown`, puis ferme les sessions ouvertes.
pub async fn run(config: &Config, shutdown: CancellationToken) -> Result<(), WorkerError> {
    let bind = config.sftp.bind.ok_or(WorkerError::Disabled)?;
    let host_key = hostkey::load(&host_key_path(config))?;
    tracing::info!(fingerprint = %hostkey::fingerprint(&host_key), "sftp host key loaded");
    let listener = TcpListener::bind(bind)
        .await
        .map_err(|source| WorkerError::Bind { addr: bind, source })?;
    serve(
        listener,
        Arc::new(ssh_config(host_key)),
        config.sftp.socket.clone(),
        config.servers.data_root.clone(),
        Duration::from_secs(u64::from(kuro_protocol::limits::SFTP_LEASE_SECS / 2)),
        shutdown,
    )
    .await;
    Ok(())
}

/// Boucle d'acceptation, séparée de la configuration pour les tests. Les baux sont renouvelés
/// toutes les `renew_every` (à mi-bail en production).
pub async fn serve(
    listener: TcpListener,
    ssh: Arc<SshConfig>,
    socket: PathBuf,
    data_root: PathBuf,
    renew_every: Duration,
    shutdown: CancellationToken,
) {
    let client = DaemonClient::spawn(socket, shutdown.clone());
    let (audit, events) = sessions::AuditQueue::new();
    let sessions = Sessions::new(audit);
    // Le transfert d'audit survit à `shutdown` le temps de remettre les dernières fermetures.
    let audit_done = CancellationToken::new();
    let forwarder = tokio::spawn(sessions::forward_audit(
        events,
        client.clone(),
        audit_done.clone(),
    ));
    tokio::spawn(
        sessions
            .clone()
            .maintain(client.clone(), renew_every, shutdown.clone()),
    );
    let shared = ssh::Shared {
        client,
        sessions: sessions.clone(),
        data_root,
    };
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    if let Ok(addr) = listener.local_addr() {
        tracing::info!(%addr, "sftp listening");
    }
    loop {
        let accepted = tokio::select! {
            () = shutdown.cancelled() => break,
            accepted = listener.accept() => accepted,
        };
        let (stream, peer) = match accepted {
            Ok(accepted) => accepted,
            Err(error) => {
                tracing::warn!(%error, "failed to accept sftp connection");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let Ok(slot) = Arc::clone(&slots).acquire_owned().await else {
            break;
        };
        let ssh = Arc::clone(&ssh);
        let shared = shared.clone();
        tokio::spawn(async move {
            let _slot = slot;
            ssh::serve(ssh, shared, stream, Some(peer)).await;
        });
    }
    // Arrêt : sessions fermées, résumés remis au daemon (quelques secondes au plus).
    sessions.close_all(CloseReason::Shutdown);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !sessions.is_empty() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    audit_done.cancel();
    let _ = forwarder.await;
}
