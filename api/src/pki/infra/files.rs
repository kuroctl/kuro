use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use kuro_runtime::fs as fs_write;

use crate::config::DaemonLinkConfig;
use crate::pki::domain::{CertificateAuthority, PkiError};

#[derive(Debug, thiserror::Error)]
pub enum PkiStoreError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("CA not found at {0}: run `kuro-api pki init` first")]
    MissingCa(PathBuf),
    #[error(transparent)]
    Pki(#[from] PkiError),
}

/// Emplacements des fichiers PKI, tirés de la section `[daemon]` de la configuration.
#[derive(Debug, Clone)]
pub struct PkiFiles {
    pub ca_cert: PathBuf,
    pub ca_key: PathBuf,
    pub server_cert: PathBuf,
    pub server_key: PathBuf,
}

impl From<&DaemonLinkConfig> for PkiFiles {
    fn from(config: &DaemonLinkConfig) -> Self {
        Self {
            ca_cert: config.ca_cert.clone(),
            ca_key: config.ca_key.clone(),
            server_cert: config.cert.clone(),
            server_key: config.key.clone(),
        }
    }
}

impl PkiFiles {
    pub fn ca_exists(&self) -> bool {
        self.ca_cert.exists()
    }

    pub fn server_exists(&self) -> bool {
        self.server_cert.exists()
    }

    pub fn load_ca(&self) -> Result<CertificateAuthority, PkiStoreError> {
        if !self.ca_exists() {
            return Err(PkiStoreError::MissingCa(self.ca_cert.clone()));
        }
        let cert = read(&self.ca_cert)?;
        let key = read(&self.ca_key)?;
        Ok(CertificateAuthority::from_pem(&cert, &key)?)
    }

    pub fn save_ca(&self, ca: &CertificateAuthority) -> Result<(), PkiStoreError> {
        write(&self.ca_cert, ca.cert_pem(), fs_write::write_file)?;
        write(&self.ca_key, &ca.key_pem(), fs_write::write_secret)
    }

    pub fn save_server(&self, chain_pem: &str, key_pem: &str) -> Result<(), PkiStoreError> {
        write(&self.server_cert, chain_pem, fs_write::write_file)?;
        write(&self.server_key, key_pem, fs_write::write_secret)
    }
}

fn read(path: &Path) -> Result<String, PkiStoreError> {
    fs::read_to_string(path).map_err(|source| io_error(path, source))
}

fn write(
    path: &Path,
    content: &str,
    writer: fn(&Path, &str) -> io::Result<()>,
) -> Result<(), PkiStoreError> {
    writer(path, content).map_err(|source| io_error(path, source))
}

fn io_error(path: &Path, source: io::Error) -> PkiStoreError {
    PkiStoreError::Io {
        path: path.to_owned(),
        source,
    }
}
