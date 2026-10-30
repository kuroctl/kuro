//! Acceptation des connexions : TCP → TLS → HTTP/1.1 (axum), avec le certificat client
//! du pair attaché à chaque requête.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{ConnectInfo, FromRef};
use axum::routing::get;
use axum::{Extension, Router};
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::service::TowerToHyperService;
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

use super::registry::LinkRegistry;
use super::{DAEMON_PATH, PeerCertificate, tls, upgrade};
use crate::config::DaemonLinkConfig;
use crate::enrollment;
use crate::enrollment::http::EnrollmentState;
use crate::http::policy::HttpPolicy;
use crate::nodes::application::Presence;
use crate::nodes::domain::CertFingerprint;
use crate::servers::domain::ServerEvents;
use crate::system::Readiness;

/// Délai maximal de la poignée de main TLS et de la lecture des en-têtes HTTP.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Tentatives d'enrôlement par minute et par adresse : un nœud s'enrôle une fois.
const ENROLL_RATE_LIMIT_PER_MINUTE: u32 = 30;

#[derive(Debug, thiserror::Error)]
pub enum LinkSetupError {
    #[error(transparent)]
    Tls(#[from] kuro_runtime::tls::TlsError),
    #[error("invalid TLS configuration: {0}")]
    Rustls(#[from] rustls::Error),
    #[error("invalid client certificate verifier: {0}")]
    Verifier(#[from] rustls::server::VerifierBuilderError),
    #[error("failed to bind daemon listener: {0}")]
    Bind(#[from] std::io::Error),
}

/// Dépendances partagées par toutes les sessions daemon.
pub(super) struct LinkContext {
    pub(super) presence: Presence,
    pub(super) registry: Arc<LinkRegistry>,
    /// Serveurs du nœud : `server.sync` après le `hello`, états rapportés.
    pub(super) servers: Arc<dyn ServerEvents>,
    pub(super) hello_timeout: Duration,
    pub(super) heartbeat_interval_secs: u64,
    pub(super) heartbeat_timeout_secs: u64,
}

/// État des routes du listener.
#[derive(Clone)]
pub(super) struct ListenerState {
    pub(super) link: Arc<LinkContext>,
    pub(super) enrollment: EnrollmentState,
    pub(super) shutdown: CancellationToken,
}

impl FromRef<ListenerState> for EnrollmentState {
    fn from_ref(state: &ListenerState) -> Self {
        state.enrollment.clone()
    }
}

pub struct DaemonListener {
    tcp: TcpListener,
    acceptor: TlsAcceptor,
    link: Arc<LinkContext>,
    enrollment: EnrollmentState,
    readiness: Readiness,
}

impl DaemonListener {
    pub async fn bind(
        config: &DaemonLinkConfig,
        presence: Presence,
        enrollment: EnrollmentState,
        links: Arc<LinkRegistry>,
        servers: Arc<dyn ServerEvents>,
    ) -> Result<Self, LinkSetupError> {
        let acceptor = TlsAcceptor::from(tls::server_config(config)?);
        let tcp = TcpListener::bind(config.bind).await?;
        let link = Arc::new(LinkContext {
            presence,
            registry: links,
            servers,
            hello_timeout: config.hello_timeout(),
            heartbeat_interval_secs: config.heartbeat_interval_secs,
            heartbeat_timeout_secs: config.heartbeat_timeout_secs,
        });
        Ok(Self {
            tcp,
            acceptor,
            link,
            enrollment,
            readiness: Readiness::default(),
        })
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.tcp.local_addr()
    }

    /// État du listener, lu par `/api/ready` : en marche de `run` jusqu'à l'arrêt.
    pub fn readiness(&self) -> Readiness {
        self.readiness.clone()
    }

    /// Accepte les connexions jusqu'à l'annulation de `shutdown`, qui ferme aussi les sessions.
    pub async fn run(self, shutdown: CancellationToken) {
        // Les pairs sont joints directement : l'adresse cliente est le pair TCP, et
        // l'enrôlement (sans authentification) est limité en débit par adresse.
        let policy = HttpPolicy::direct(ENROLL_RATE_LIMIT_PER_MINUTE);
        let router = Router::new()
            .route(DAEMON_PATH, get(upgrade::daemon_socket))
            .merge(enrollment::http::routes())
            .fallback(crate::http::extract::not_found)
            .with_state(ListenerState {
                link: self.link,
                enrollment: self.enrollment,
                shutdown: shutdown.clone(),
            })
            .layer(axum::middleware::from_fn(crate::http::trace::log_request))
            .layer(axum::middleware::from_fn_with_state(
                policy,
                crate::http::policy::apply,
            ));

        if let Ok(addr) = self.tcp.local_addr() {
            tracing::info!(%addr, "daemon listener ready");
        }
        self.readiness.listener_up();
        loop {
            let accepted = tokio::select! {
                () = shutdown.cancelled() => break,
                accepted = self.tcp.accept() => accepted,
            };
            let (stream, peer) = match accepted {
                Ok(accepted) => accepted,
                Err(error) => {
                    // Typiquement une limite de descripteurs atteinte : on laisse respirer.
                    tracing::warn!(%error, "failed to accept daemon connection");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            };
            tokio::spawn(
                serve_connection(stream, peer, self.acceptor.clone(), router.clone())
                    .instrument(tracing::info_span!("daemon_connection", %peer)),
            );
        }
        self.readiness.listener_down();
    }
}

async fn serve_connection(
    stream: TcpStream,
    peer_addr: SocketAddr,
    acceptor: TlsAcceptor,
    router: Router,
) {
    let tls = match timeout(HANDSHAKE_TIMEOUT, acceptor.accept(stream)).await {
        Ok(Ok(tls)) => tls,
        Ok(Err(error)) => {
            tracing::warn!(%error, "tls handshake failed");
            return;
        }
        Err(_) => {
            tracing::warn!("tls handshake timed out");
            return;
        }
    };

    let peer = PeerCertificate(
        tls.get_ref()
            .1
            .peer_certificates()
            .and_then(|chain| chain.first())
            .map(|cert| CertFingerprint::from_der(cert)),
    );
    // Comme pour le REST : certificat et adresse du pair, identifiant de requête (audit).
    let router = router
        .layer(Extension(peer))
        .layer(Extension(ConnectInfo(peer_addr)));
    let service = TowerToHyperService::new(crate::http::with_request_id(router));

    let connection = hyper::server::conn::http1::Builder::new()
        .timer(TokioTimer::new())
        .header_read_timeout(HANDSHAKE_TIMEOUT)
        .serve_connection(TokioIo::new(tls), service)
        .with_upgrades();
    if let Err(error) = connection.await {
        tracing::debug!(%error, "daemon connection ended with an error");
    }
}
