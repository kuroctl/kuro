//! Proxy des sites web (ADR-0038) sur le Docker de la machine : un vrai Caddy, certificat de
//! l'autorité interne, un site relayé vers le conteneur de son serveur, puis retiré.

use std::collections::BTreeMap;
use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;

use kuro_daemon::docker::Docker;
use kuro_daemon::install::download::DownloadPolicy;
use kuro_daemon::server::Manager;
use kuro_daemon::web::{DEFAULT_IMAGE, TlsMode, WebConfig};
use kuro_protocol::api::server::{
    LifecycleSpec, Limits, PortProtocol, PortSpec, PowerAction, ServerDeploy, ServiceSpec,
    StopSignal, StopSpec, WebRoute,
};
use kuro_protocol::daemon::server::{CertificateState, ProxyState, ServerState};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::TlsConnector;
use uuid::Uuid;

use super::support::{Node, wait_for};

const HOST: &str = "abcdefg234.web.kuroctl.test";

/// Port libre de l'hôte, rendu aussitôt : le proxy le prendra.
fn free_port() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.local_addr().unwrap().port()
}

/// Nœud dont le proxy écoute sur des ports libres, avec un nom de conteneur à lui.
fn web_node() -> (Node, WebConfig) {
    let data = tempfile::TempDir::new().unwrap();
    let docker = Docker::connect(super::support::IMAGE).unwrap();
    let web = WebConfig {
        http_port: free_port(),
        https_port: free_port(),
        image: DEFAULT_IMAGE.to_owned(),
        tls: TlsMode::Internal,
        acme_email: None,
        max_body_bytes: 1024 * 1024,
        container: format!("kuro-proxy-test-{}", Uuid::now_v7().simple()),
    };
    let (manager, outgoing) = Manager::with_options(
        docker,
        data.path().to_path_buf(),
        DownloadPolicy::default(),
        web.clone(),
    );
    (
        Node {
            manager,
            events: outgoing.events,
            streams: outgoing.streams,
            backups: outgoing.backups,
            data,
        },
        web,
    )
}

/// Image BusyBox officielle : son `httpd` sert un dossier, sans root.
const BACKEND_IMAGE: &str = "docker.io/library/busybox:1.36.1";

/// Un serveur dont le service `web` répond « ok from backend » sur son port 8080.
fn site(server_id: Uuid) -> ServerDeploy {
    ServerDeploy {
        server_id,
        services: vec![ServiceSpec {
            name: "web".to_owned(),
            image: BACKEND_IMAGE.to_owned(),
            command: vec![
                "sh".to_owned(),
                "-c".to_owned(),
                "echo 'ok from backend' > /tmp/index.html && exec httpd -f -p 8080 -h /tmp"
                    .to_owned(),
            ],
            env: BTreeMap::new(),
            ports: vec![PortSpec {
                name: "http".to_owned(),
                container_port: 8080,
                protocol: PortProtocol::Tcp,
                published: None,
            }],
            volumes: Vec::new(),
            writable: vec!["/tmp".to_owned()],
            user: "1000:1000".to_owned(),
            limits: Limits {
                memory_bytes: 64 * 1024 * 1024,
                cpu_millis: 500,
                pids: 64,
            },
            depends_on: Vec::new(),
            healthcheck: None,
        }],
        files: Vec::new(),
        lifecycle: LifecycleSpec {
            service: "web".to_owned(),
            ready: None,
            stop: StopSpec::Signal(StopSignal::Term),
            stop_timeout_secs: 2,
        },
        install: None,
        disk_bytes: 0,
        suspended: false,
        defer: false,
        web: vec![WebRoute {
            host: HOST.to_owned(),
            service: "web".to_owned(),
            container_port: 8080,
        }],
    }
}

/// Accepte tout certificat : l'autorité interne de Caddy n'est connue de personne.
#[derive(Debug)]
struct TrustAnything;

impl ServerCertVerifier for TrustAnything {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        kuro_runtime::tls::provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// `GET /` en HTTPS sur le proxy, pour `host` : statut et corps, ou l'erreur de poignée de main.
async fn https_get(port: u16, host: &str) -> Result<(u16, String), String> {
    let config = rustls::ClientConfig::builder_with_provider(kuro_runtime::tls::provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(TrustAnything))
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(config));
    let tcp = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .map_err(|e| e.to_string())?;
    let name = ServerName::try_from(host.to_owned()).unwrap();
    let mut tls = connector
        .connect(name, tcp)
        .await
        .map_err(|e| format!("handshake: {e}"))?;
    tls.write_all(
        format!("GET / HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes(),
    )
    .await
    .map_err(|e| e.to_string())?;
    let mut response = Vec::new();
    let _ = tls.read_to_end(&mut response).await;
    let text = String::from_utf8_lossy(&response).into_owned();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("no status line in {text:?}"))?;
    let body = text.split("\r\n\r\n").nth(1).unwrap_or_default().to_owned();
    Ok((status, body))
}

/// Réessaie jusqu'à ce que la requête réussisse avec le corps attendu, ou expire.
async fn wait_for_body(port: u16, host: &str, expected: &str) -> Result<(u16, String), String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let mut last = Err("not tried".to_owned());
    while tokio::time::Instant::now() < deadline {
        last = https_get(port, host).await;
        if matches!(&last, Ok((200, body)) if body.trim() == expected) {
            return last;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    last
}

#[tokio::test]
async fn a_web_server_is_served_over_https_by_the_node_proxy_then_unplugged() {
    let (mut node, web) = web_node();
    let id = Uuid::now_v7();
    let bollard = bollard::Docker::connect_with_defaults().unwrap();

    node.manager.deploy(site(id)).unwrap();
    wait_for(&mut node.events, id, ServerState::Offline).await;
    node.manager.power(id, PowerAction::Start).await.unwrap();
    wait_for(&mut node.events, id, ServerState::Running).await;

    // Le proxy tourne, durci, sur les ports configurés, relié au réseau du serveur.
    let inspect = bollard
        .inspect_container(&web.container, None)
        .await
        .unwrap();
    let host_config = inspect.host_config.unwrap();
    assert_eq!(
        inspect.config.as_ref().unwrap().user.as_deref(),
        Some("65532:65532")
    );
    assert_eq!(host_config.readonly_rootfs, Some(true));
    assert_eq!(host_config.cap_drop, Some(vec!["ALL".to_owned()]));
    let bindings = host_config.port_bindings.unwrap();
    assert_eq!(
        bindings["8443/tcp"].as_ref().unwrap()[0]
            .host_port
            .as_deref(),
        Some(web.https_port.to_string().as_str())
    );
    let networks = inspect.network_settings.unwrap().networks.unwrap();
    assert!(networks.contains_key(&kuro_daemon::docker::network_name(id)));
    // Aucun port de site publié sur l'hôte.
    let site_container = bollard
        .inspect_container(&kuro_daemon::docker::container_name(id, "web"), None)
        .await
        .unwrap();
    assert!(
        site_container
            .host_config
            .unwrap()
            .port_bindings
            .unwrap_or_default()
            .is_empty()
    );

    // Le site répond en HTTPS, par son nom, avec le corps du conteneur du serveur.
    let (status, body) = wait_for_body(web.https_port, HOST, "ok from backend")
        .await
        .unwrap();
    assert_eq!(status, 200);
    assert_eq!(body.trim(), "ok from backend");
    // Un autre nom n'a pas de certificat : la poignée de main échoue.
    assert!(
        https_get(web.https_port, "other.web.kuroctl.test")
            .await
            .is_err()
    );

    // L'état web accompagne les états du serveur : proxy prêt, certificat obtenu (interne).
    node.manager.sync(vec![id]);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut status = None;
    while tokio::time::Instant::now() < deadline {
        if let Some(report) = node.events.try_recv()
            && report.server_id == id
        {
            status = report.web;
            if status
                .as_ref()
                .is_some_and(|w| w.certificate == CertificateState::Issued)
            {
                break;
            }
        }
        // Le daemon relit les certificats à intervalle régulier ; ici on force la relecture.
        node.manager.refresh_web_status().await;
        node.manager.sync(vec![id]);
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    let status = status.expect("web status reported");
    assert_eq!(status.proxy, ProxyState::Ready, "{status:?}");
    assert_eq!(status.certificate, CertificateState::Issued, "{status:?}");

    // Serveur supprimé : plus de route, le proxy a quitté son réseau, puis s'arrête (dernier site).
    node.manager.delete(id).unwrap();
    wait_for(&mut node.events, id, ServerState::Deleted).await;
    assert!(
        bollard
            .inspect_network(&kuro_daemon::docker::network_name(id), None)
            .await
            .is_err()
    );
    assert!(
        bollard
            .inspect_container(&web.container, None)
            .await
            .is_err(),
        "the proxy leaves with the last site"
    );
    assert!(https_get(web.https_port, HOST).await.is_err());
}
