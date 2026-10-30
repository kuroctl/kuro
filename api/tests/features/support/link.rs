//! Listener daemon de test, nœuds enrôlés par le vrai client du daemon, et clients « bruts »
//! (WebSocket et HTTPS) pour piloter le protocole à la main.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::header::{CONTENT_TYPE, HOST};
use hyper::{Request, StatusCode};
use hyper_util::rt::TokioIo;
use kuro_api::audit::domain::AuditContext;
use kuro_api::config::DaemonLinkConfig;
use kuro_api::enrollment::application::{NodeTicket, add_node};
use kuro_api::enrollment::http::EnrollmentState;
use kuro_api::nodes::application::{Instance, PlacementInput, Presence};
use kuro_api::nodes::domain::NodeId;
use kuro_api::nodes::infra::NodeRepository;
use kuro_api::ws::{DAEMON_PATH, DaemonListener, LinkRegistry};
use kuro_daemon::config::{ApiConfig, Config as DaemonConfig, TlsConfig};
use kuro_daemon::enroll::{EnrollError, EnrollOptions, Enrolled};
use kuro_daemon::link::WsStream;
use kuro_protocol::enroll::ENROLL_PATH;
use kuro_runtime::telemetry::LogConfig;
use rustls::ClientConfig;
use rustls::pki_types::ServerName;
use serde_json::{Value, json};
use sqlx::PgPool;
use tempfile::TempDir;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

use super::db;
use super::pki::{NodeFiles, TestPki};

/// Délais courts pour garder des tests rapides.
pub const HELLO_TIMEOUT_SECS: u64 = 1;
pub const HEARTBEAT_INTERVAL_SECS: u64 = 1;
pub const HEARTBEAT_TIMEOUT_SECS: u64 = 2;
const TOKEN_TTL: Duration = Duration::from_secs(15 * 60);

/// Délai d'attente maximal d'un événement attendu.
const WAIT: Duration = Duration::from_secs(10);

/// Listener daemon démarré sur un port libre, avec sa base et sa CA. Arrêté au drop.
pub struct TestApi {
    pub addr: SocketAddr,
    pub pool: PgPool,
    pub pki: TestPki,
    /// Instance de l'API que ce listener représente (ADR-0037).
    pub instance: Instance,
    /// Registre des sessions du listener, comme celui que reçoit le REST.
    pub links: Arc<LinkRegistry>,
    /// Spectateurs des consoles, partagés avec le REST.
    pub console: kuro_api::servers::application::ConsoleHub,
    /// Événements du lien, à rattacher aux cas d'usage du REST (redéploiement après extraction).
    pub events: Arc<kuro_api::servers::application::LinkEvents>,
    hosts: TempDir,
    shutdown: CancellationToken,
}

/// Nœud enrôlé : sa configuration de daemon est prête à l'emploi.
pub struct TestNode {
    pub id: NodeId,
    pub config: DaemonConfig,
}

impl TestApi {
    pub async fn start() -> Self {
        Self::start_with_heartbeat_timeout(HEARTBEAT_TIMEOUT_SECS).await
    }

    /// Délai de heartbeat long : pour des tests qui pilotent un daemon simulé sans heartbeat.
    pub async fn start_with_heartbeat_timeout(heartbeat_timeout_secs: u64) -> Self {
        let pool = db::fresh_pool().await;
        let pki = TestPki::new();
        let config = DaemonLinkConfig {
            bind: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
            ca_cert: pki.files.ca_cert.clone(),
            ca_key: pki.files.ca_key.clone(),
            cert: pki.files.server_cert.clone(),
            key: pki.files.server_key.clone(),
            hello_timeout_secs: HELLO_TIMEOUT_SECS,
            heartbeat_interval_secs: HEARTBEAT_INTERVAL_SECS,
            heartbeat_timeout_secs,
        };
        let instance = Instance::start(pool.clone()).await.unwrap();
        let presence = Presence::new(NodeRepository::new(pool.clone()), instance.clone());
        let enrollment = EnrollmentState {
            pool: pool.clone(),
            ca: Arc::new(pki.ca()),
            certificate_validity: time::Duration::days(1),
        };
        let links = Arc::new(LinkRegistry::default());
        let console = kuro_api::servers::application::ConsoleHub::new(links.clone());
        let events = Arc::new(kuro_api::servers::application::LinkEvents::new(
            pool.clone(),
            console.clone(),
            links.clone(),
        ));
        let listener =
            DaemonListener::bind(&config, presence, enrollment, links.clone(), events.clone())
                .await
                .unwrap();
        let addr = listener.local_addr().unwrap();

        let shutdown = CancellationToken::new();
        tokio::spawn(listener.run(shutdown.clone()));
        Self {
            addr,
            pool,
            pki,
            instance,
            links,
            console,
            events,
            hosts: tempfile::tempdir().unwrap(),
            shutdown,
        }
    }

    pub fn url(&self) -> String {
        format!("https://127.0.0.1:{}", self.addr.port())
    }

    /// `kuro-api node add` : nœud `pending` et son token.
    pub async fn add_node(&self, name: &str) -> NodeTicket {
        add_node(
            &self.pool,
            &AuditContext::cli("test"),
            name,
            PlacementInput::default(),
            TOKEN_TTL,
        )
        .await
        .unwrap()
    }

    /// Emplacement de la configuration d'un hôte de test (un dossier par hôte).
    pub fn host_config_path(&self, host: &str) -> PathBuf {
        self.hosts.path().join(host).join("daemon.toml")
    }

    /// `kuro-daemonctl enroll`, depuis l'hôte `host`.
    pub async fn enroll_from(
        &self,
        host: &str,
        token: &str,
        fingerprint: &str,
    ) -> Result<Enrolled, EnrollError> {
        kuro_daemon::enroll::enroll(&EnrollOptions {
            api_url: self.url(),
            ca_fingerprint: fingerprint.to_owned(),
            token: token.to_owned(),
            config_path: self.host_config_path(host),
        })
        .await
    }

    /// Nœud ajouté puis enrôlé par le vrai client du daemon.
    pub async fn enroll_node(&self, name: &str) -> TestNode {
        let ticket = self.add_node(name).await;
        let enrolled = self
            .enroll_from(name, ticket.token.expose(), &self.pki.fingerprint)
            .await
            .unwrap();
        TestNode {
            id: NodeId::from(ticket.node.id),
            config: DaemonConfig::load(&enrolled.config_path).unwrap(),
        }
    }

    /// Configuration de daemon pointant sur ce listener, avec des fichiers donnés.
    pub fn daemon_config(&self, files: &NodeFiles) -> DaemonConfig {
        DaemonConfig {
            api: ApiConfig {
                url: format!("wss://127.0.0.1:{}{DAEMON_PATH}", self.addr.port()),
            },
            tls: TlsConfig {
                ca_cert: files.ca_cert.clone(),
                cert: files.cert.clone(),
                key: files.key.clone(),
            },
            log: LogConfig::default(),
            servers: kuro_daemon::config::ServersConfig::default(),
            web: Default::default(),
        }
    }

    /// Connexion mTLS établie, aucun message envoyé.
    pub async fn connect(&self, node: &TestNode) -> WsStream {
        kuro_daemon::link::connect(&node.config).await.unwrap()
    }

    pub async fn status(&self, id: impl Into<uuid::Uuid>) -> String {
        sqlx::query_scalar("SELECT status FROM nodes WHERE id = $1")
            .bind(id.into())
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    pub async fn wait_for_status(&self, id: impl Into<uuid::Uuid> + Copy, expected: &str) {
        let id: uuid::Uuid = id.into();
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            let status = self.status(id).await;
            if status == expected {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "node {id} stayed {status}, expected {expected}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// `POST /api/enroll` à la main, avec ou sans certificat client.
    pub async fn post_enroll(
        &self,
        body: Value,
        client: Option<&NodeFiles>,
    ) -> (StatusCode, Value) {
        let builder = ClientConfig::builder_with_provider(kuro_runtime::tls::provider())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .with_root_certificates(
                kuro_runtime::tls::root_store(&self.pki.files.ca_cert).unwrap(),
            );
        let config = match client {
            Some(files) => builder
                .with_client_auth_cert(
                    kuro_runtime::tls::load_certs(&files.cert).unwrap(),
                    kuro_runtime::tls::load_private_key(&files.key).unwrap(),
                )
                .unwrap(),
            None => builder.with_no_client_auth(),
        };
        let tcp = TcpStream::connect(self.addr).await.unwrap();
        let tls = TlsConnector::from(Arc::new(config))
            .connect(ServerName::try_from("127.0.0.1").unwrap(), tcp)
            .await
            .unwrap();
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
            .await
            .unwrap();
        tokio::spawn(connection);

        let request = Request::post(ENROLL_PATH)
            .header(HOST, "127.0.0.1")
            .header(CONTENT_TYPE, "application/json")
            .body(Full::new(Bytes::from(body.to_string())))
            .unwrap();
        let response = sender.send_request(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
}

impl Drop for TestApi {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

/// Tentative de connexion TLS + WebSocket sans certificat client.
pub async fn connect_without_client_cert(api: &TestApi) -> Result<(), String> {
    let config = ClientConfig::builder_with_provider(kuro_runtime::tls::provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_root_certificates(kuro_runtime::tls::root_store(&api.pki.files.ca_cert).unwrap())
        .with_no_client_auth();
    let tcp = TcpStream::connect(api.addr)
        .await
        .map_err(|e| e.to_string())?;
    let tls = TlsConnector::from(Arc::new(config))
        .connect(ServerName::try_from("127.0.0.1").unwrap(), tcp)
        .await
        .map_err(|e| e.to_string())?;
    let url = format!("wss://127.0.0.1:{}{DAEMON_PATH}", api.addr.port());
    tokio_tungstenite::client_async(url, tls)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn hello(id: &str, protocol_version: u16) -> Value {
    json!({
        "id": id,
        "type": "hello",
        "payload": {
            "protocol_version": protocol_version,
            "daemon_version": env!("CARGO_PKG_VERSION"),
            "host": { "os": "linux", "arch": "x86_64", "cpu_cores": 4, "memory_bytes": 8_589_934_592_u64 }
        }
    })
}

pub fn heartbeat() -> Value {
    json!({ "id": uuid::Uuid::now_v7(), "type": "heartbeat", "payload": {} })
}

pub async fn send(ws: &mut WsStream, message: Value) {
    ws.send(Message::text(message.to_string())).await.unwrap();
}

/// Prochain message texte, ou `None` si la connexion se ferme.
pub async fn recv(ws: &mut WsStream) -> Option<Value> {
    tokio::time::timeout(WAIT, async {
        while let Some(frame) = ws.next().await {
            match frame {
                Ok(Message::Text(text)) => {
                    return Some(serde_json::from_str(text.as_str()).unwrap());
                }
                Ok(Message::Close(_)) | Err(_) => return None,
                Ok(_) => {}
            }
        }
        None
    })
    .await
    .expect("no message nor close received in time")
}

/// Envoie un `hello` valide et retourne le `hello.ack`, après avoir lu le `server.sync` qui
/// le suit.
pub async fn say_hello(ws: &mut WsStream) -> Value {
    let id = uuid::Uuid::now_v7().to_string();
    send(ws, hello(&id, kuro_protocol::PROTOCOL_VERSION)).await;
    let ack = recv(ws).await.expect("connection closed before hello.ack");
    assert_eq!(ack["type"], "hello.ack", "unexpected reply: {ack}");
    let sync = recv(ws)
        .await
        .expect("connection closed before server.sync");
    assert_eq!(sync["type"], "server.sync", "unexpected message: {sync}");
    ack
}

pub async fn assert_closed(ws: &mut WsStream) {
    if let Some(message) = recv(ws).await {
        panic!("expected the connection to close, got {message}");
    }
}

/// Fichiers d'un nœud déjà enrôlé, pour rejouer une requête avec son certificat.
pub fn node_files(node: &TestNode) -> NodeFiles {
    NodeFiles {
        ca_cert: node.config.tls.ca_cert.clone(),
        cert: node.config.tls.cert.clone(),
        key: node.config.tls.key.clone(),
    }
}
