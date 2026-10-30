use rcgen::{
    BasicConstraints, CertificateParams, CertificateSigningRequestParams, DnType,
    ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair, KeyUsagePurpose, SerialNumber,
};
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;
use time::{Duration, OffsetDateTime};

/// Durée de vie de la CA.
const CA_VALIDITY: Duration = Duration::days(3650);
/// Durée de vie du certificat serveur du listener daemon.
const SERVER_VALIDITY: Duration = Duration::days(730);
/// Tolérance de décalage d'horloge entre l'API et les nœuds.
const CLOCK_SKEW: Duration = Duration::minutes(5);

#[derive(Debug, thiserror::Error)]
pub enum PkiError {
    /// CSR illisible, extension non supportée ou signature invalide.
    #[error("invalid certificate signing request: {0}")]
    InvalidCsr(rcgen::Error),
    #[error("invalid CA certificate PEM")]
    InvalidCaCertificate,
    #[error(transparent)]
    Rcgen(#[from] rcgen::Error),
    #[error(transparent)]
    Random(#[from] kuro_runtime::tls::TlsError),
}

/// Certificat émis avec sa clé privée (certificat serveur de l'API uniquement).
#[derive(Debug)]
pub struct IssuedCertificate {
    /// Certificat suivi de celui de la CA : le daemon y retrouve la CA qu'il a épinglée.
    pub chain_pem: String,
    pub key_pem: String,
}

/// Certificat signé à partir d'une CSR : la clé privée reste chez le demandeur.
#[derive(Debug)]
pub struct SignedCertificate {
    pub cert_pem: String,
    pub der: CertificateDer<'static>,
}

#[derive(Debug)]
pub struct CertificateAuthority {
    cert_pem: String,
    cert_der: CertificateDer<'static>,
    issuer: Issuer<'static, KeyPair>,
}

impl CertificateAuthority {
    pub fn generate(common_name: &str) -> Result<Self, PkiError> {
        let mut params = CertificateParams::new(Vec::<String>::new())?;
        params
            .distinguished_name
            .push(DnType::CommonName, common_name);
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        set_validity(&mut params, CA_VALIDITY);
        let key = KeyPair::generate()?;
        let cert = params.self_signed(&key)?;
        Ok(Self {
            cert_pem: cert.pem(),
            cert_der: cert.der().clone(),
            issuer: Issuer::new(params, key),
        })
    }

    pub fn from_pem(cert_pem: &str, key_pem: &str) -> Result<Self, PkiError> {
        let cert_der = CertificateDer::from_pem_slice(cert_pem.as_bytes())
            .map_err(|_| PkiError::InvalidCaCertificate)?;
        let key = KeyPair::from_pem(key_pem)?;
        Ok(Self {
            cert_pem: cert_pem.to_owned(),
            cert_der,
            issuer: Issuer::from_ca_cert_pem(cert_pem, key)?,
        })
    }

    pub fn cert_pem(&self) -> &str {
        &self.cert_pem
    }

    pub fn key_pem(&self) -> String {
        self.issuer.key().serialize_pem()
    }

    /// Empreinte à communiquer aux opérateurs pour l'épinglage côté daemon.
    pub fn fingerprint(&self) -> String {
        kuro_runtime::tls::fingerprint(&self.cert_der)
    }

    /// Certificat serveur du listener daemon. `names` : noms DNS ou adresses IP.
    pub fn issue_server(&self, names: &[String]) -> Result<IssuedCertificate, PkiError> {
        let mut params = CertificateParams::new(names.to_vec())?;
        params
            .distinguished_name
            .push(DnType::CommonName, "kuro-api");
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        self.prepare_leaf(&mut params, SERVER_VALIDITY)?;
        let key = KeyPair::generate()?;
        let cert = params.signed_by(&key, &self.issuer)?;
        Ok(IssuedCertificate {
            chain_pem: format!("{}{}", cert.pem(), self.cert_pem),
            key_pem: key.serialize_pem(),
        })
    }

    /// Signe la CSR d'un nœud. Seule la clé publique est reprise : le nom (CN) et les usages
    /// sont imposés par la CA, quoi que demande la CSR.
    pub fn sign_node_csr(
        &self,
        csr_pem: &str,
        node_name: &str,
        validity: Duration,
    ) -> Result<SignedCertificate, PkiError> {
        let csr =
            CertificateSigningRequestParams::from_pem(csr_pem).map_err(PkiError::InvalidCsr)?;
        let mut params = CertificateParams::new(Vec::<String>::new())?;
        params
            .distinguished_name
            .push(DnType::CommonName, node_name);
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        self.prepare_leaf(&mut params, validity)?;
        let cert = params.signed_by(&csr.public_key, &self.issuer)?;
        Ok(SignedCertificate {
            cert_pem: cert.pem(),
            der: cert.der().clone(),
        })
    }

    fn prepare_leaf(
        &self,
        params: &mut CertificateParams,
        validity: Duration,
    ) -> Result<(), PkiError> {
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.use_authority_key_identifier_extension = true;
        // Numéro de série aléatoire : unique, il servira à la révocation.
        let mut serial = kuro_runtime::tls::random_bytes::<16>()?;
        serial[0] &= 0x7f;
        params.serial_number = Some(SerialNumber::from_slice(&serial));
        set_validity(params, validity);
        Ok(())
    }
}

fn set_validity(params: &mut CertificateParams, validity: Duration) {
    let now = OffsetDateTime::now_utc();
    params.not_before = now - CLOCK_SKEW;
    params.not_after = now + validity;
}
