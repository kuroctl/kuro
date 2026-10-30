//! Briques rustls communes : fournisseur crypto, aléa, empreintes et lecture des fichiers PEM.

use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rustls::RootCertStore;
use rustls::crypto::CryptoProvider;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("failed to read PEM file {path}: {source}")]
    Pem {
        path: PathBuf,
        #[source]
        source: rustls::pki_types::pem::Error,
    },
    #[error("no certificate found in {0}")]
    NoCertificate(PathBuf),
    #[error(transparent)]
    Rustls(#[from] rustls::Error),
    #[error("secure random generator unavailable")]
    Random,
}

/// Fournisseur crypto unique du projet. Passé explicitement à chaque config rustls
/// pour ne jamais dépendre du fournisseur « par défaut » du processus.
pub fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Octets aléatoires issus du générateur cryptographique du fournisseur.
pub fn random_bytes<const N: usize>() -> Result<[u8; N], TlsError> {
    let mut bytes = [0u8; N];
    provider()
        .secure_random
        .fill(&mut bytes)
        .map_err(|_| TlsError::Random)?;
    Ok(bytes)
}

/// Encodage hexadécimal minuscule.
pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        // L'écriture dans une String ne peut pas échouer.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Empreinte d'un certificat : SHA-256 de son DER, en hexadécimal minuscule.
/// Format unique partagé par l'API (identité des nœuds) et le daemon (épinglage de la CA).
pub fn fingerprint(der: &[u8]) -> String {
    sha256_hex(der)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// Normalise une empreinte saisie par un opérateur : préfixe `sha256:` facultatif,
/// séparateurs `:` et majuscules tolérés. `None` si ce n'est pas un SHA-256.
pub fn parse_fingerprint(input: &str) -> Option<String> {
    let input = input.trim();
    let input = input.strip_prefix("sha256:").unwrap_or(input);
    let hex: String = input
        .chars()
        .filter(|c| *c != ':')
        .map(|c| c.to_ascii_lowercase())
        .collect();
    (hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit())).then_some(hex)
}

/// Chaîne de certificats d'un fichier PEM (au moins un certificat).
pub fn load_certs(path: &Path) -> Result<Vec<CertificateDer<'static>>, TlsError> {
    let certs = CertificateDer::pem_file_iter(path)
        .and_then(|iter| iter.collect::<Result<Vec<_>, _>>())
        .map_err(|source| pem_error(path, source))?;
    if certs.is_empty() {
        return Err(TlsError::NoCertificate(path.to_owned()));
    }
    Ok(certs)
}

pub fn load_private_key(path: &Path) -> Result<PrivateKeyDer<'static>, TlsError> {
    PrivateKeyDer::from_pem_file(path).map_err(|source| pem_error(path, source))
}

/// Magasin de confiance limité aux certificats du fichier (la CA privée).
pub fn root_store(ca_path: &Path) -> Result<RootCertStore, TlsError> {
    let mut store = RootCertStore::empty();
    for cert in load_certs(ca_path)? {
        store.add(cert)?;
    }
    Ok(store)
}

fn pem_error(path: &Path, source: rustls::pki_types::pem::Error) -> TlsError {
    TlsError::Pem {
        path: path.to_owned(),
        source,
    }
}
