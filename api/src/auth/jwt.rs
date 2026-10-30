//! JWT HS256 minimal (ADR-0007) : seul l'algorithme HS256 est accepté, l'en-tête est comparé
//! octet pour octet, la signature en temps constant, puis `iss`, `aud` et `exp` sont vérifiés.
//!
//! Écrit à la main parce que `jsonwebtoken` impose `aws-lc-rs` ou `rsa` 0.9 (RUSTSEC-2023-0071).

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use uuid::Uuid;

const ISSUER: &str = "kuro";
/// `{"alg":"HS256","typ":"JWT"}` encodé : tout autre en-tête est refusé.
const HEADER: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9";
/// Tolérance de décalage d'horloge sur `iat`.
const CLOCK_SKEW_SECS: u64 = 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    pub iss: String,
    pub aud: String,
    pub sub: Uuid,
    /// Session (famille de jetons de rafraîchissement) pour les jetons d'accès.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sid: Option<Uuid>,
    pub iat: u64,
    pub exp: u64,
}

impl Claims {
    pub fn new(
        audience: &str,
        subject: Uuid,
        session: Option<Uuid>,
        now: u64,
        ttl_secs: u64,
    ) -> Self {
        Self {
            iss: ISSUER.to_owned(),
            aud: audience.to_owned(),
            sub: subject,
            sid: session,
            iat: now,
            exp: now + ttl_secs,
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum JwtError {
    #[error("malformed token")]
    Malformed,
    #[error("invalid token signature")]
    Signature,
    #[error("token issued for another audience")]
    Audience,
    #[error("token expired")]
    Expired,
}

pub fn encode(key: &[u8; 32], claims: &Claims) -> String {
    // La sérialisation d'une structure de champs simples ne peut pas échouer.
    let payload = serde_json::to_vec(claims).unwrap_or_default();
    let signing_input = format!("{HEADER}.{}", URL_SAFE_NO_PAD.encode(payload));
    let signature = URL_SAFE_NO_PAD.encode(sign(key, &signing_input).finalize().into_bytes());
    format!("{signing_input}.{signature}")
}

pub fn decode(key: &[u8; 32], token: &str, audience: &str, now: u64) -> Result<Claims, JwtError> {
    let mut parts = token.split('.');
    let (Some(header), Some(payload), Some(signature), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(JwtError::Malformed);
    };
    if header != HEADER {
        return Err(JwtError::Malformed);
    }

    let signature = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| JwtError::Malformed)?;
    sign(key, &format!("{header}.{payload}"))
        .verify_slice(&signature)
        .map_err(|_| JwtError::Signature)?;

    let payload = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| JwtError::Malformed)?;
    let claims: Claims = serde_json::from_slice(&payload).map_err(|_| JwtError::Malformed)?;
    if claims.iss != ISSUER || claims.aud != audience {
        return Err(JwtError::Audience);
    }
    if claims.exp <= now || claims.iat > now + CLOCK_SKEW_SECS {
        return Err(JwtError::Expired);
    }
    Ok(claims)
}

fn sign(key: &[u8; 32], input: &str) -> Hmac<Sha256> {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key)
        .unwrap_or_else(|_| unreachable!("hmac accepts keys of any length"));
    mac.update(input.as_bytes());
    mac
}
