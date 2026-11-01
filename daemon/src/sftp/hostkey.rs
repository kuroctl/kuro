//! Clé d'hôte SSH du nœud (ed25519) : générée par le daemon au premier démarrage, lue par le
//! processus SFTP. Son empreinte est annoncée à l'API dans `hello` et affichée dans le panel.

use std::path::Path;

use ring::rand::{SecureRandom, SystemRandom};
use russh::keys::ssh_key::LineEnding;
use russh::keys::ssh_key::private::{Ed25519Keypair, KeypairData};
use russh::keys::{HashAlg, PrivateKey};

#[derive(Debug, thiserror::Error)]
pub enum HostKeyError {
    #[error("{path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid host key: {0}")]
    Key(String),
    #[error("system random generator failed")]
    Random,
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> HostKeyError + '_ {
    move |source| HostKeyError::Io {
        path: path.display().to_string(),
        source,
    }
}

/// Lit la clé, ou la crée (`0600`) si elle n'existe pas encore.
pub fn ensure(path: &Path) -> Result<PrivateKey, HostKeyError> {
    match std::fs::read_to_string(path) {
        Ok(text) => return parse(&text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(io(path)(error)),
    }
    let mut seed = [0u8; 32];
    SystemRandom::new()
        .fill(&mut seed)
        .map_err(|_| HostKeyError::Random)?;
    let key = PrivateKey::new(
        KeypairData::Ed25519(Ed25519Keypair::from_seed(&seed)),
        "kuro-sftp",
    )
    .map_err(|e| HostKeyError::Key(e.to_string()))?;
    let text = key
        .to_openssh(LineEnding::LF)
        .map_err(|e| HostKeyError::Key(e.to_string()))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io(parent))?;
    }
    write_private(path, text.as_bytes())?;
    Ok(key)
}

pub fn load(path: &Path) -> Result<PrivateKey, HostKeyError> {
    parse(&std::fs::read_to_string(path).map_err(io(path))?)
}

fn parse(text: &str) -> Result<PrivateKey, HostKeyError> {
    PrivateKey::from_openssh(text).map_err(|e| HostKeyError::Key(e.to_string()))
}

/// `SHA256:…`, comme OpenSSH.
pub fn fingerprint(key: &PrivateKey) -> String {
    key.public_key().fingerprint(HashAlg::Sha256).to_string()
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), HostKeyError> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(io(path))?;
    file.write_all(bytes).map_err(io(path))?;
    file.sync_all().map_err(io(path))
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), HostKeyError> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(io(path))?;
    file.write_all(bytes).map_err(io(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_key_is_created_once_then_reused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys").join("host_ed25519");
        let created = ensure(&path).unwrap();
        let reused = ensure(&path).unwrap();
        assert_eq!(fingerprint(&created), fingerprint(&reused));
        assert!(fingerprint(&created).starts_with("SHA256:"));
        assert_eq!(fingerprint(&load(&path).unwrap()), fingerprint(&created));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
