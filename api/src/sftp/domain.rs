//! Règles SFTP indépendantes de la base : nom d'utilisateur et clés publiques SSH (ADR-0047).

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use sha2::{Digest, Sha256};

/// Longueur maximale d'un identifiant de serveur ou de compte dans un nom d'utilisateur.
const MAX_HANDLE_LEN: usize = 32;
/// Taille maximale d'une clé publique collée dans le panel.
pub const MAX_PUBLIC_KEY_CHARS: usize = 8 * 1024;
/// Nom d'une clé (libellé choisi par l'admin).
pub const MAX_KEY_NAME_CHARS: usize = 100;

/// `<identifiant du serveur>.<identifiant du compte>`, ou `None` si la forme est fausse.
pub fn parse_username(username: &str) -> Option<(&str, &str)> {
    let (server, account) = username.split_once('.')?;
    (valid_handle(server) && valid_handle(account)).then_some((server, account))
}

fn valid_handle(handle: &str) -> bool {
    !handle.is_empty()
        && handle.len() <= MAX_HANDLE_LEN
        && handle
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

pub fn username(server_handle: &str, account_handle: &str) -> String {
    format!("{server_handle}.{account_handle}")
}

/// Algorithmes acceptés : ceux que le daemon vérifie sans la crate `rsa` (ADR-0047).
const ACCEPTED: &[&str] = &[
    "ssh-ed25519",
    "ecdsa-sha2-nistp256",
    "ecdsa-sha2-nistp384",
    "ecdsa-sha2-nistp521",
];
/// Reconnus mais refusés, avec un message explicite.
const REFUSED: &[&str] = &["ssh-rsa", "ssh-dss"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    #[error("not an OpenSSH public key")]
    Invalid,
    #[error("unsupported key type")]
    Unsupported,
}

/// Clé publique normalisée : algorithme, blob du protocole SSH (base64) et empreinte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicKey {
    pub algorithm: String,
    /// Base64 standard du blob, comparé tel quel à celui que présente le client.
    pub blob: String,
    /// `SHA256:…` (base64 sans remplissage), comme OpenSSH.
    pub fingerprint: String,
}

/// Lit une ligne `<algorithme> <base64> [commentaire]` (format OpenSSH, celui qu'affiche aussi
/// PuTTYgen). Le commentaire est ignoré ; le blob doit annoncer le même algorithme et être bien
/// formé pour son type.
pub fn parse_public_key(text: &str) -> Result<PublicKey, KeyError> {
    if text.len() > MAX_PUBLIC_KEY_CHARS {
        return Err(KeyError::Invalid);
    }
    let mut parts = text.split_whitespace();
    let (Some(algorithm), Some(encoded)) = (parts.next(), parts.next()) else {
        return Err(KeyError::Invalid);
    };
    if REFUSED.contains(&algorithm) || algorithm.starts_with("sk-") {
        return Err(KeyError::Unsupported);
    }
    if !ACCEPTED.contains(&algorithm) {
        return Err(KeyError::Invalid);
    }
    let blob = STANDARD.decode(encoded).map_err(|_| KeyError::Invalid)?;
    from_blob(algorithm, &blob)
}

/// Clé présentée par un client SSH : blob en base64, algorithme annoncé.
pub fn from_wire(algorithm: &str, encoded: &str) -> Result<PublicKey, KeyError> {
    let blob = STANDARD.decode(encoded).map_err(|_| KeyError::Invalid)?;
    from_blob(algorithm, &blob)
}

fn from_blob(algorithm: &str, blob: &[u8]) -> Result<PublicKey, KeyError> {
    let mut reader = Reader(blob);
    if reader.string()? != algorithm.as_bytes() {
        return Err(KeyError::Invalid);
    }
    match algorithm {
        "ssh-ed25519" => {
            if reader.string()?.len() != 32 {
                return Err(KeyError::Invalid);
            }
        }
        _ => {
            // `ecdsa-sha2-<courbe>` : nom de la courbe, puis point non compressé.
            let curve = algorithm
                .strip_prefix("ecdsa-sha2-")
                .ok_or(KeyError::Unsupported)?;
            if reader.string()? != curve.as_bytes() {
                return Err(KeyError::Invalid);
            }
            let point = reader.string()?;
            let expected = match curve {
                "nistp256" => 65,
                "nistp384" => 97,
                "nistp521" => 133,
                _ => return Err(KeyError::Unsupported),
            };
            if point.len() != expected || point.first() != Some(&0x04) {
                return Err(KeyError::Invalid);
            }
        }
    }
    if !reader.0.is_empty() {
        return Err(KeyError::Invalid);
    }
    Ok(PublicKey {
        algorithm: algorithm.to_owned(),
        blob: STANDARD.encode(blob),
        fingerprint: fingerprint(blob),
    })
}

pub fn fingerprint(blob: &[u8]) -> String {
    format!("SHA256:{}", STANDARD_NO_PAD.encode(Sha256::digest(blob)))
}

/// Lecture des chaînes du format SSH (longueur sur 4 octets, puis contenu).
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn string(&mut self) -> Result<&'a [u8], KeyError> {
        let (length, rest) = self.0.split_first_chunk::<4>().ok_or(KeyError::Invalid)?;
        let length = usize::try_from(u32::from_be_bytes(*length)).map_err(|_| KeyError::Invalid)?;
        if rest.len() < length {
            return Err(KeyError::Invalid);
        }
        let (value, rest) = rest.split_at(length);
        self.0 = rest;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Clé ed25519 générée par `ssh-keygen` (publique, sans valeur).
    const ED25519: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl test@kuro";

    #[test]
    fn usernames() {
        assert_eq!(parse_username("a1b2c3.k7f9x2"), Some(("a1b2c3", "k7f9x2")));
        assert_eq!(parse_username("root"), None);
        assert_eq!(parse_username("A1.b2"), None);
        assert_eq!(parse_username(".b2"), None);
        assert_eq!(parse_username("a.b.c"), None);
        assert_eq!(parse_username(&format!("{}.b", "a".repeat(33))), None);
    }

    #[test]
    fn ed25519_key_is_normalized() {
        let key = parse_public_key(ED25519).unwrap();
        assert_eq!(key.algorithm, "ssh-ed25519");
        assert_eq!(
            key.blob,
            "AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl"
        );
        assert!(key.fingerprint.starts_with("SHA256:"));
        assert!(!key.fingerprint.ends_with('='));
        // Sans commentaire, même clé.
        let bare = parse_public_key(
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl",
        )
        .unwrap();
        assert_eq!(bare, key);
        assert_eq!(from_wire("ssh-ed25519", &key.blob).unwrap(), key);
    }

    #[test]
    fn malformed_keys_are_refused() {
        assert_eq!(parse_public_key(""), Err(KeyError::Invalid));
        assert_eq!(parse_public_key("ssh-ed25519"), Err(KeyError::Invalid));
        assert_eq!(parse_public_key("ssh-ed25519 !!!"), Err(KeyError::Invalid));
        // Blob d'un autre algorithme que celui annoncé.
        let blob = STANDARD.encode([0, 0, 0, 3, b'a', b'b', b'c']);
        assert_eq!(
            parse_public_key(&format!("ssh-ed25519 {blob}")),
            Err(KeyError::Invalid)
        );
        // Octets en trop après la clé.
        let mut blob = STANDARD
            .decode("AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl")
            .unwrap();
        blob.push(0);
        assert_eq!(
            parse_public_key(&format!("ssh-ed25519 {}", STANDARD.encode(blob))),
            Err(KeyError::Invalid)
        );
    }

    #[test]
    fn rsa_is_refused_explicitly() {
        assert_eq!(
            parse_public_key("ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQ== me"),
            Err(KeyError::Unsupported)
        );
        assert_eq!(
            parse_public_key("sk-ssh-ed25519@openssh.com AAAA me"),
            Err(KeyError::Unsupported)
        );
    }
}
