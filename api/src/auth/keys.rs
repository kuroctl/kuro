//! Clés dérivées de `auth.secret_key` : une clé par usage, pour qu'aucune ne serve à deux choses.

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

const MASTER_KEY_BYTES: usize = 32;

#[derive(Debug, thiserror::Error)]
#[error("auth.secret_key must be 32 bytes encoded in hexadecimal (64 characters)")]
pub struct InvalidSecretKey;

/// Clés de travail. Jamais affichées, même en debug.
pub struct AuthKeys {
    jwt: [u8; 32],
    totp: [u8; 32],
    server_secrets: [u8; 32],
    session_successors: [u8; 32],
}

impl AuthKeys {
    pub fn from_hex(master_hex: &str) -> Result<Self, InvalidSecretKey> {
        let master = decode_hex(master_hex.trim()).ok_or(InvalidSecretKey)?;
        if master.len() != MASTER_KEY_BYTES {
            return Err(InvalidSecretKey);
        }
        Ok(Self {
            jwt: derive(&master, b"kuro/jwt/v1"),
            totp: derive(&master, b"kuro/totp/v1"),
            server_secrets: derive(&master, b"kuro/server-secrets/v1"),
            session_successors: derive(&master, b"kuro/session-successor/v1"),
        })
    }

    pub fn jwt(&self) -> &[u8; 32] {
        &self.jwt
    }

    pub fn totp(&self) -> &[u8; 32] {
        &self.totp
    }

    /// Réglages `secret` des serveurs (mots de passe générés des prefabs).
    pub fn server_secrets(&self) -> &[u8; 32] {
        &self.server_secrets
    }

    /// Successeur d'un jeton de rafraîchissement, gardé chiffré le temps de la grâce.
    pub fn session_successors(&self) -> &[u8; 32] {
        &self.session_successors
    }
}

impl std::fmt::Debug for AuthKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AuthKeys(<redacted>)")
    }
}

fn derive(master: &[u8], label: &[u8]) -> [u8; 32] {
    // HMAC accepte une clé de toute longueur : `new_from_slice` ne peut pas échouer.
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(master)
        .unwrap_or_else(|_| unreachable!("hmac accepts keys of any length"));
    mac.update(label);
    mac.finalize().into_bytes().into()
}

fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect()
}
