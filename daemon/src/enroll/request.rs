//! `POST /api/enroll` en HTTPS, sur une connexion dont la CA a été vérifiée.

use std::sync::Arc;
use std::time::Duration;

use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Bytes;
use hyper::header::{CONTENT_TYPE, HOST};
use hyper::{Request, StatusCode};
use hyper_util::rt::TokioIo;
use kuro_protocol::enroll::{ENROLL_PATH, EnrollRequest, EnrollResponse};
use kuro_runtime::tls;
use rustls::ClientConfig;
use rustls::pki_types::ServerName;
use serde::Deserialize;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;

use super::pinned::PinnedCaVerifier;
use super::{EnrollError, Target};

const STEP_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

/// Corps d'erreur problem+json de l'API : seul le code est exploité ici.
#[derive(Deserialize)]
struct Problem {
    code: String,
}

pub(super) async fn post(
    target: &Target,
    ca_fingerprint: &str,
    body: &EnrollRequest,
) -> Result<EnrollResponse, EnrollError> {
    let provider = tls::provider();
    let mut config = ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedCaVerifier::new(ca_fingerprint, provider)))
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];

    let server_name = ServerName::try_from(target.host.clone())
        .map_err(|_| EnrollError::InvalidUrl(target.authority.clone()))?;
    let tcp = timeout(
        STEP_TIMEOUT,
        TcpStream::connect((target.host.as_str(), target.port)),
    )
    .await
    .map_err(|_| EnrollError::Timeout("connecting"))??;
    // Si l'empreinte ne correspond pas, la poignée de main échoue ici : le token n'est jamais envoyé.
    let stream = timeout(
        STEP_TIMEOUT,
        TlsConnector::from(Arc::new(config)).connect(server_name, tcp),
    )
    .await
    .map_err(|_| EnrollError::Timeout("negotiating TLS"))??;

    let (mut sender, connection) =
        hyper::client::conn::http1::handshake(TokioIo::new(stream)).await?;
    tokio::spawn(async move {
        if let Err(error) = connection.await {
            tracing::debug!(%error, "enrollment connection ended with an error");
        }
    });

    let request = Request::post(ENROLL_PATH)
        .header(HOST, &target.authority)
        .header(CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from(serde_json::to_vec(body)?)))?;
    let response = timeout(STEP_TIMEOUT, sender.send_request(request))
        .await
        .map_err(|_| EnrollError::Timeout("waiting for the api"))??;

    let status = response.status();
    let bytes = Limited::new(response.into_body(), MAX_RESPONSE_BYTES)
        .collect()
        .await
        .map_err(|_| EnrollError::ResponseBody)?
        .to_bytes();

    match status {
        StatusCode::OK => Ok(serde_json::from_slice(&bytes)?),
        StatusCode::UNAUTHORIZED => Err(EnrollError::InvalidToken),
        status => Err(EnrollError::Rejected {
            status: status.as_u16(),
            code: serde_json::from_slice::<Problem>(&bytes)
                .map(|body| body.code)
                .unwrap_or_else(|_| String::from_utf8_lossy(&bytes).into_owned()),
        }),
    }
}
