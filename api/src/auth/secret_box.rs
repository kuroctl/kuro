//! Chiffrement authentifié des secrets stockés en base (secrets TOTP) : XChaCha20-Poly1305,
//! nonce aléatoire de 192 bits placé en tête du message chiffré.

use chacha20poly1305::aead::Aead;
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};

const NONCE_BYTES: usize = 24;

#[derive(Debug, thiserror::Error)]
pub enum SecretBoxError {
    #[error("failed to encrypt secret")]
    Seal,
    /// Données altérées ou clé différente.
    #[error("failed to decrypt secret")]
    Open,
    #[error(transparent)]
    Random(#[from] kuro_runtime::tls::TlsError),
}

pub fn seal(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, SecretBoxError> {
    let nonce = kuro_runtime::tls::random_bytes::<NONCE_BYTES>()?;
    let cipher = XChaCha20Poly1305::new(key.into());
    let ciphertext = cipher
        .encrypt(&XNonce::from(nonce), plaintext)
        .map_err(|_| SecretBoxError::Seal)?;
    let mut sealed = Vec::with_capacity(NONCE_BYTES + ciphertext.len());
    sealed.extend_from_slice(&nonce);
    sealed.extend_from_slice(&ciphertext);
    Ok(sealed)
}

pub fn open(key: &[u8; 32], sealed: &[u8]) -> Result<Vec<u8>, SecretBoxError> {
    let (nonce, ciphertext) = sealed
        .split_at_checked(NONCE_BYTES)
        .ok_or(SecretBoxError::Open)?;
    let nonce: [u8; NONCE_BYTES] = nonce.try_into().map_err(|_| SecretBoxError::Open)?;
    XChaCha20Poly1305::new(key.into())
        .decrypt(&XNonce::from(nonce), ciphertext)
        .map_err(|_| SecretBoxError::Open)
}
