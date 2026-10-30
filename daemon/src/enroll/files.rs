//! Fichiers produits par l'enrôlement, rangés à côté de la configuration du daemon.

use std::path::{Path, PathBuf};

use kuro_runtime::fs::{write_file, write_secret};

use super::EnrollError;
use crate::config::Config;

#[derive(Debug)]
pub(super) struct NodeFiles {
    pub(super) config: PathBuf,
    pub(super) ca_cert: PathBuf,
    pub(super) cert: PathBuf,
    pub(super) key: PathBuf,
}

impl NodeFiles {
    pub(super) fn beside(config: &Path) -> Self {
        let dir = config.parent().unwrap_or(Path::new("."));
        Self {
            config: config.to_owned(),
            ca_cert: dir.join("ca.pem"),
            cert: dir.join("node.pem"),
            key: dir.join("node.key"),
        }
    }

    /// Un hôte déjà enrôlé n'est jamais écrasé : sa clé privée serait perdue.
    pub(super) fn ensure_absent(&self) -> Result<(), EnrollError> {
        for path in [&self.key, &self.config] {
            if path.exists() {
                return Err(EnrollError::AlreadyEnrolled(path.clone()));
            }
        }
        Ok(())
    }

    pub(super) fn write(
        &self,
        ca_pem: &str,
        cert_pem: &str,
        key_pem: &str,
        config: &Config,
    ) -> Result<(), EnrollError> {
        let config_toml = format!(
            "# Écrit par `kuro-daemonctl enroll`.\n\n{}",
            toml::to_string(config)?
        );
        write(&self.key, key_pem, write_secret)?;
        write(&self.cert, cert_pem, write_file)?;
        write(&self.ca_cert, ca_pem, write_file)?;
        write(&self.config, &config_toml, write_file)
    }
}

fn write(
    path: &Path,
    content: &str,
    writer: fn(&Path, &str) -> std::io::Result<()>,
) -> Result<(), EnrollError> {
    writer(path, content).map_err(|source| EnrollError::Write {
        path: path.to_owned(),
        source,
    })
}
