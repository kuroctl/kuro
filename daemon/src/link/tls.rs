//! Configuration rustls du client : TLS 1.3, seule la CA privée fait foi, certificat du nœud.

use std::sync::Arc;

use kuro_runtime::tls;
use rustls::ClientConfig;

use super::LinkError;
use crate::config::TlsConfig;

pub(super) fn client_config(config: &TlsConfig) -> Result<Arc<ClientConfig>, LinkError> {
    let client = ClientConfig::builder_with_provider(tls::provider())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_root_certificates(tls::root_store(&config.ca_cert)?)
        .with_client_auth_cert(
            tls::load_certs(&config.cert)?,
            tls::load_private_key(&config.key)?,
        )?;
    Ok(Arc::new(client))
}
