//! Téléchargement d'une étape `download` (ADR-0017) : HTTPS seulement, adresses publiques
//! seulement (à chaque redirection), taille bornée, empreinte (SHA-256 ou SHA-512) calculée au
//! fil de l'eau.
//!
//! Le nom est résolu une fois, chaque adresse est vérifiée, puis la connexion part vers l'adresse
//! vérifiée : une seconde résolution DNS ne peut pas rediriger vers le réseau interne de l'hôte.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use http_body_util::{BodyExt, Empty};
use hyper::body::Bytes;
use hyper::header::{HOST, LOCATION, USER_AGENT};
use hyper::{Request, StatusCode, Uri};
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, RootCertStore};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;

use super::InstallError;

/// Taille maximale d'un téléchargement.
pub(crate) const MAX_DOWNLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Sans donnée reçue pendant ce délai, le téléchargement est abandonné.
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Règles du téléchargeur. En production : autorités publiques (webpki), adresses publiques.
#[derive(Debug, Clone)]
pub struct DownloadPolicy {
    pub roots: Arc<RootCertStore>,
    /// Tests uniquement : autorise un serveur local.
    pub allow_private: bool,
    pub max_bytes: u64,
}

impl Default for DownloadPolicy {
    fn default() -> Self {
        Self {
            roots: Arc::new(RootCertStore {
                roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
            }),
            allow_private: false,
            max_bytes: MAX_DOWNLOAD_BYTES,
        }
    }
}

fn failed(message: impl Into<String>) -> InstallError {
    InstallError::Download(message.into())
}

/// Empreinte attendue d'un téléchargement, en hexadécimal.
#[derive(Debug, Clone, Copy)]
pub enum Expected<'a> {
    /// Étapes d'installation des prefabs (ADR-0017).
    Sha256(&'a str),
    /// Fichiers des providers de contenu (ADR-0020) : Modrinth.
    Sha512(&'a str),
    /// CurseForge, qui ne publie que SHA-1.
    Sha1(&'a str),
}

impl<'a> Expected<'a> {
    /// Empreinte d'un fichier de contenu, dans l'algorithme publié par sa source.
    pub fn of(hash: &'a kuro_protocol::api::ContentHash) -> Self {
        match hash {
            kuro_protocol::api::ContentHash::Sha512(value) => Self::Sha512(value),
            kuro_protocol::api::ContentHash::Sha1(value) => Self::Sha1(value),
        }
    }
}

/// Télécharge `url` dans `out` et vérifie son empreinte ; retourne la taille. En cas d'erreur,
/// l'appelant jette ce qui a été écrit.
pub async fn fetch(
    policy: &DownloadPolicy,
    url: &str,
    expected: Expected<'_>,
    out: &mut (impl AsyncWrite + Unpin),
) -> Result<u64, InstallError> {
    let mut uri: Uri = url.parse().map_err(|_| failed("invalid url"))?;
    for _ in 0..=MAX_REDIRECTS {
        let response = get(policy, &uri).await?;
        let status = response.status();
        if status.is_redirection() {
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| failed(format!("redirect without location ({status})")))?;
            uri = redirect(&uri, location)?;
            continue;
        }
        if status != StatusCode::OK {
            return Err(failed(format!("http status {status}")));
        }
        let digests = stream(policy, response.into_body(), out).await?;
        let (wanted, actual) = match expected {
            Expected::Sha256(wanted) => (wanted, digests.sha256),
            Expected::Sha512(wanted) => (wanted, digests.sha512),
            Expected::Sha1(wanted) => (wanted, digests.sha1),
        };
        if !actual.eq_ignore_ascii_case(wanted) {
            return Err(InstallError::Checksum {
                expected: wanted.to_owned(),
                actual,
            });
        }
        return Ok(digests.size);
    }
    Err(failed("too many redirects"))
}

async fn get(
    policy: &DownloadPolicy,
    uri: &Uri,
) -> Result<hyper::Response<hyper::body::Incoming>, InstallError> {
    if uri.scheme_str() != Some("https") {
        return Err(failed("only https downloads are allowed"));
    }
    let host = uri.host().ok_or_else(|| failed("url without host"))?;
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned();
    let port = uri.port_u16().unwrap_or(443);
    let address = public_address(policy, &host, port).await?;

    let mut config = ClientConfig::builder_with_provider(kuro_runtime::tls::provider())
        .with_safe_default_protocol_versions()
        .map_err(|e| failed(e.to_string()))?
        .with_root_certificates(policy.roots.clone())
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let server_name = ServerName::try_from(host.clone()).map_err(|_| failed("invalid host"))?;

    let tcp = timeout(CONNECT_TIMEOUT, TcpStream::connect(address))
        .await
        .map_err(|_| failed("connection timed out"))?
        .map_err(|e| failed(e.to_string()))?;
    let tls = timeout(
        CONNECT_TIMEOUT,
        TlsConnector::from(Arc::new(config)).connect(server_name, tcp),
    )
    .await
    .map_err(|_| failed("tls handshake timed out"))?
    .map_err(|e| failed(e.to_string()))?;
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
        .await
        .map_err(|e| failed(e.to_string()))?;
    tokio::spawn(async move {
        if let Err(error) = connection.await {
            tracing::debug!(%error, "download connection ended with an error");
        }
    });

    let authority = uri.authority().map_or(host.as_str(), |a| a.as_str());
    let path = uri.path_and_query().map_or("/", |p| p.as_str());
    let request = Request::get(path)
        .header(HOST, authority)
        .header(
            USER_AGENT,
            concat!("kuro-daemon/", env!("CARGO_PKG_VERSION")),
        )
        .body(Empty::<Bytes>::new())
        .map_err(|e| failed(e.to_string()))?;
    timeout(CONNECT_TIMEOUT, sender.send_request(request))
        .await
        .map_err(|_| failed("no response"))?
        .map_err(|e| failed(e.to_string()))
}

/// Adresse vérifiée à laquelle se connecter. Toutes les adresses du nom doivent être publiques :
/// une réponse mixte est refusée.
async fn public_address(
    policy: &DownloadPolicy,
    host: &str,
    port: u16,
) -> Result<SocketAddr, InstallError> {
    let addresses: Vec<SocketAddr> = match host.parse::<IpAddr>() {
        Ok(ip) => vec![SocketAddr::new(ip, port)],
        Err(_) => tokio::net::lookup_host((host, port))
            .await
            .map_err(|e| failed(format!("cannot resolve {host}: {e}")))?
            .collect(),
    };
    let first = *addresses
        .first()
        .ok_or_else(|| failed(format!("{host} has no address")))?;
    if !policy.allow_private
        && let Some(refused) = addresses.iter().find(|a| !is_public(a.ip()))
    {
        return Err(failed(format!(
            "{host} resolves to a non-public address ({})",
            refused.ip()
        )));
    }
    Ok(first)
}

fn redirect(base: &Uri, location: &str) -> Result<Uri, InstallError> {
    let target: Uri = location.parse().map_err(|_| failed("invalid redirect"))?;
    if target.scheme().is_some() {
        return Ok(target);
    }
    // Redirection relative : même origine.
    let mut parts = base.clone().into_parts();
    parts.path_and_query = target.path_and_query().cloned();
    Uri::from_parts(parts).map_err(|_| failed("invalid redirect"))
}

/// Taille et empreintes d'un téléchargement, calculées au fil de l'eau.
struct Digests {
    size: u64,
    sha256: String,
    sha512: String,
    sha1: String,
}

async fn stream(
    policy: &DownloadPolicy,
    mut body: hyper::body::Incoming,
    out: &mut (impl AsyncWrite + Unpin),
) -> Result<Digests, InstallError> {
    let mut hasher = Sha256::new();
    let mut hasher512 = Sha512::new();
    let mut hasher1 = Sha1::new();
    let mut size: u64 = 0;
    loop {
        let frame = match timeout(IDLE_TIMEOUT, body.frame()).await {
            Err(_) => return Err(failed("download stalled")),
            Ok(None) => break,
            Ok(Some(frame)) => frame.map_err(|e| failed(e.to_string()))?,
        };
        let Ok(data) = frame.into_data() else {
            continue;
        };
        size = size.saturating_add(data.len() as u64);
        if size > policy.max_bytes {
            return Err(failed(format!("larger than {} bytes", policy.max_bytes)));
        }
        hasher.update(&data);
        hasher512.update(&data);
        hasher1.update(&data);
        out.write_all(&data)
            .await
            .map_err(|e| failed(e.to_string()))?;
    }
    out.flush().await.map_err(|e| failed(e.to_string()))?;
    Ok(Digests {
        size,
        sha256: kuro_runtime::tls::hex(&hasher.finalize()),
        sha512: kuro_runtime::tls::hex(&hasher512.finalize()),
        sha1: kuro_runtime::tls::hex(&hasher1.finalize()),
    })
}

/// Adresse joignable sur Internet : ni boucle locale, ni réseau privé, lien local (métadonnées
/// cloud), CGNAT, multicast, plages réservées ou de documentation.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_public_v4(v4),
            None => is_public_v6(v6),
        },
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_multicast()
        || a == 0
        || (a == 100 && (64..128).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 198 && (18..20).contains(&b))
        || a >= 240)
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || (first & 0xfe00) == 0xfc00
        || (first & 0xffc0) == 0xfe80
        || first == 0x2001 && ip.segments()[1] == 0x0db8
        || first == 0x0064 && ip.segments()[1] == 0xff9b)
}
