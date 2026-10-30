//! Configuration rustls du listener : TLS 1.3, certificat client facultatif mais, s'il est
//! présenté, obligatoirement signé par la CA. Les routes décident ensuite de l'exiger ou non.

use std::sync::Arc;

use kuro_runtime::tls;
use rustls::ServerConfig;
use rustls::server::WebPkiClientVerifier;
use rustls::server::danger::ClientCertVerifier;

use super::LinkSetupError;
use crate::config::DaemonLinkConfig;

pub(super) fn server_config(
    config: &DaemonLinkConfig,
) -> Result<Arc<ServerConfig>, LinkSetupError> {
    let provider = tls::provider();
    let roots = Arc::new(tls::root_store(&config.ca_cert)?);
    let verifier: Arc<dyn ClientCertVerifier> =
        WebPkiClientVerifier::builder_with_provider(roots, provider.clone())
            .allow_unauthenticated()
            .build()?;

    let mut server = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_client_cert_verifier(verifier)
        .with_single_cert(
            tls::load_certs(&config.cert)?,
            tls::load_private_key(&config.key)?,
        )?;
    server.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(server))
}
