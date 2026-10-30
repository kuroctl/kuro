//! Secrets opaques à usage unique ou rotatifs (token d'enrôlement, jeton de rafraîchissement) :
//! 256 bits d'aléa préfixés, dont seul le SHA-256 est persisté.

use kuro_runtime::tls::{self, TlsError};

/// Génère `<prefix><64 caractères hexadécimaux>`.
pub fn generate(prefix: &str) -> Result<String, TlsError> {
    let secret = tls::random_bytes::<32>()?;
    Ok(format!("{prefix}{}", tls::hex(&secret)))
}

/// Forme persistée. Un SHA-256 suffit : avec 256 bits d'entropie, une fonction lente
/// (Argon2) n'apporterait rien contre la force brute.
pub fn hash(secret: &str) -> String {
    tls::sha256_hex(secret.as_bytes())
}
