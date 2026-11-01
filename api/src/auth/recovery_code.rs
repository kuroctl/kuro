//! Codes de secours du TOTP : 12 caractères base32 (60 bits), affichés `xxxx-xxxx-xxxx`.
//!
//! Seul un HMAC-SHA256 est persisté, avec une clé dérivée de `auth.secret_key` : 60 bits
//! d'aléa suffisent contre une recherche en ligne (verrouillage après 5 échecs), et la clé
//! empêche une recherche hors ligne à partir de la seule base.

use hmac::{Hmac, KeyInit, Mac};
use kuro_runtime::tls::{self, TlsError};
use sha2::Sha256;

/// Nombre de codes délivrés à chaque génération.
pub const COUNT: usize = 10;
const LENGTH: usize = 12;
const GROUP: usize = 4;
/// Base32 (RFC 4648) en minuscules : 32 symboles, 256 en est un multiple (aucun biais).
const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// Un code, sous sa forme canonique (12 caractères, sans tiret).
pub fn generate() -> Result<String, TlsError> {
    let bytes = tls::random_bytes::<LENGTH>()?;
    Ok(bytes
        .iter()
        .map(|byte| char::from(ALPHABET[usize::from(*byte) % ALPHABET.len()]))
        .collect())
}

/// Forme d'affichage d'un code canonique : `xxxx-xxxx-xxxx`.
pub fn display(canonical: &str) -> String {
    let mut code = String::with_capacity(LENGTH + LENGTH / GROUP);
    for (index, c) in canonical.chars().enumerate() {
        if index > 0 && index % GROUP == 0 {
            code.push('-');
        }
        code.push(c);
    }
    code
}

/// Forme canonique d'une saisie : tirets et espaces retirés, minuscules. `None` si ce ne peut
/// pas être un code de secours.
pub fn normalize(input: &str) -> Option<String> {
    let code: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_lowercase())
        .collect();
    (code.len() == LENGTH && code.bytes().all(|b| ALPHABET.contains(&b))).then_some(code)
}

/// Empreinte persistée d'un code canonique.
pub fn hash(key: &[u8; 32], canonical: &str) -> Vec<u8> {
    // HMAC accepte une clé de toute longueur : `new_from_slice` ne peut pas échouer.
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key)
        .unwrap_or_else(|_| unreachable!("hmac accepts keys of any length"));
    mac.update(canonical.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_codes_are_displayed_in_groups_and_read_back() {
        let canonical = generate().unwrap();
        let shown = display(&canonical);
        assert_eq!(shown.len(), 14);
        assert_eq!(shown.matches('-').count(), 2);
        assert_eq!(normalize(&shown), Some(canonical));
    }

    #[test]
    fn normalization_ignores_case_dashes_and_spaces() {
        assert_eq!(
            normalize(" ABCD-efgh 2345 ").as_deref(),
            Some("abcdefgh2345")
        );
    }

    #[test]
    fn totp_codes_and_garbage_are_not_recovery_codes() {
        assert_eq!(normalize("123456"), None);
        assert_eq!(normalize("abcd-efgh-234"), None);
        assert_eq!(normalize("abcd-efgh-2341"), None);
    }

    #[test]
    fn hash_depends_on_the_key() {
        assert_ne!(
            hash(&[1; 32], "abcdefgh2345"),
            hash(&[2; 32], "abcdefgh2345")
        );
        assert_eq!(
            hash(&[1; 32], "abcdefgh2345"),
            hash(&[1; 32], "abcdefgh2345")
        );
    }
}
