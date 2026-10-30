//! Vérification du serveur par l'empreinte de sa CA (épinglage).
//!
//! Le serveur présente sa chaîne `[certificat, CA]`. On y cherche la CA dont l'empreinte
//! correspond, puis on valide normalement la chaîne avec cette seule racine (signature,
//! validité, nom du serveur). Aucune autre autorité n'est acceptée.

use std::sync::Arc;

use kuro_runtime::tls;
use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, RootCertStore, SignatureScheme};

#[derive(Debug)]
pub(super) struct PinnedCaVerifier {
    fingerprint: String,
    provider: Arc<CryptoProvider>,
}

impl PinnedCaVerifier {
    pub(super) fn new(fingerprint: &str, provider: Arc<CryptoProvider>) -> Self {
        Self {
            fingerprint: fingerprint.to_owned(),
            provider,
        }
    }
}

impl ServerCertVerifier for PinnedCaVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let ca = intermediates
            .iter()
            .find(|cert| tls::fingerprint(cert) == self.fingerprint)
            .ok_or(rustls::Error::InvalidCertificate(
                CertificateError::UnknownIssuer,
            ))?;
        let mut roots = RootCertStore::empty();
        roots.add(ca.clone().into_owned())?;
        let verifier =
            WebPkiServerVerifier::builder_with_provider(Arc::new(roots), self.provider.clone())
                .build()
                .map_err(|error| rustls::Error::General(error.to_string()))?;
        verifier.verify_server_cert(end_entity, &[], server_name, ocsp_response, now)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}
