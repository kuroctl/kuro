//! Enrôlement de l'hôte : clé et CSR générées ici, échangées avec un token à usage unique
//! contre un certificat signé par la CA de l'API.
//!
//! Le serveur est authentifié par l'empreinte de sa CA, fournie hors bande : le token n'est
//! envoyé qu'une fois cette empreinte vérifiée.

mod files;
mod pinned;
mod request;

use std::path::PathBuf;

use kuro_protocol::enroll::EnrollRequest;
use kuro_runtime::telemetry::{LogConfig, LogFormat};
use kuro_runtime::tls;
use rcgen::{CertificateParams, KeyPair};
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;
use tokio_tungstenite::tungstenite::http::Uri;
use uuid::Uuid;

use crate::config::{ApiConfig, Config, TlsConfig};
use files::NodeFiles;

/// Variable d'environnement portant le token (jamais en argument de ligne de commande).
pub const TOKEN_ENV: &str = "KURO_ENROLL_TOKEN";

#[derive(Debug, thiserror::Error)]
pub enum EnrollError {
    #[error("invalid CA fingerprint: expected a SHA-256 in hexadecimal")]
    InvalidFingerprint,
    #[error("invalid api url {0:?}: expected https://host[:port]")]
    InvalidUrl(String),
    #[error("this host is already enrolled ({0} exists)")]
    AlreadyEnrolled(PathBuf),
    #[error("failed to generate key or CSR: {0}")]
    Csr(#[from] rcgen::Error),
    #[error(transparent)]
    Tls(#[from] tls::TlsError),
    #[error("TLS error: {0}")]
    Rustls(#[from] rustls::Error),
    #[error("connection failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("timed out while {0}")]
    Timeout(&'static str),
    #[error("http error: {0}")]
    Http(#[from] hyper::Error),
    #[error("invalid http request: {0}")]
    HttpRequest(#[from] hyper::http::Error),
    #[error("failed to read the api response")]
    ResponseBody,
    #[error("the enrollment token is invalid, expired or already used")]
    InvalidToken,
    #[error("enrollment rejected by the api ({status}): {code}")]
    Rejected { status: u16, code: String },
    #[error("unexpected api response: {0}")]
    InvalidResponse(#[from] serde_json::Error),
    #[error("the api returned a CA that does not match the pinned fingerprint")]
    CaMismatch,
    #[error("failed to write {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to render the daemon configuration: {0}")]
    Config(#[from] toml::ser::Error),
}

#[derive(Debug)]
pub struct EnrollOptions {
    /// Adresse du listener daemon de l'API, par exemple `https://panel.example.com:8443`.
    pub api_url: String,
    /// Empreinte de la CA affichée par `kuro-api pki init` / `node add`.
    pub ca_fingerprint: String,
    pub token: String,
    /// Configuration du daemon à écrire ; certificats et clé sont rangés à côté.
    pub config_path: PathBuf,
}

#[derive(Debug)]
pub struct Enrolled {
    pub node_id: Uuid,
    pub node_name: String,
    pub config_path: PathBuf,
}

pub async fn enroll(options: &EnrollOptions) -> Result<Enrolled, EnrollError> {
    let fingerprint =
        tls::parse_fingerprint(&options.ca_fingerprint).ok_or(EnrollError::InvalidFingerprint)?;
    let target = Target::parse(&options.api_url)?;
    // Chemins absolus : le service ne démarre pas forcément depuis ce dossier.
    let files = NodeFiles::beside(&std::path::absolute(&options.config_path)?);
    files.ensure_absent()?;

    let key = KeyPair::generate()?;
    let csr_pem = CertificateParams::new(Vec::<String>::new())?
        .serialize_request(&key)?
        .pem()?;

    let request = EnrollRequest {
        token: options.token.clone(),
        csr_pem,
    };
    let response = request::post(&target, &fingerprint, &request).await?;

    // La CA renvoyée doit être celle qui a été épinglée : c'est la seule racine du daemon.
    let ca_der = CertificateDer::from_pem_slice(response.ca_pem.as_bytes())
        .map_err(|_| EnrollError::CaMismatch)?;
    if tls::fingerprint(&ca_der) != fingerprint {
        return Err(EnrollError::CaMismatch);
    }

    let config = Config {
        api: ApiConfig {
            url: target.daemon_url(),
        },
        tls: TlsConfig {
            ca_cert: files.ca_cert.clone(),
            cert: files.cert.clone(),
            key: files.key.clone(),
        },
        log: LogConfig {
            format: LogFormat::Json,
            filter: "info".to_owned(),
        },
        servers: crate::config::ServersConfig::default(),
    };
    files.write(
        &response.ca_pem,
        &response.cert_pem,
        &key.serialize_pem(),
        &config,
    )?;

    Ok(Enrolled {
        node_id: response.node_id,
        node_name: response.node_name,
        config_path: files.config.clone(),
    })
}

/// Listener daemon de l'API, tel que donné par l'opérateur.
#[derive(Debug)]
struct Target {
    host: String,
    port: u16,
    authority: String,
}

impl Target {
    fn parse(url: &str) -> Result<Self, EnrollError> {
        let invalid = || EnrollError::InvalidUrl(url.to_owned());
        let uri: Uri = url.parse().map_err(|_| invalid())?;
        if uri.scheme_str() != Some("https") {
            return Err(invalid());
        }
        let authority = uri.authority().ok_or_else(invalid)?;
        Ok(Self {
            host: authority
                .host()
                .trim_start_matches('[')
                .trim_end_matches(']')
                .to_owned(),
            port: authority.port_u16().unwrap_or(443),
            authority: authority.as_str().to_owned(),
        })
    }

    /// Le lien WebSocket passe par le même listener que l'enrôlement.
    fn daemon_url(&self) -> String {
        format!("wss://{}{}", self.authority, kuro_protocol::DAEMON_PATH)
    }
}
