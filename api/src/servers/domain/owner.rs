//! Propriétaire d'un serveur (ADR-0051) : identifiant opaque posé par une extension, jamais
//! interprété par Kuro. Même contrainte que la colonne `servers.owner_ref` (migration 0057).

/// Taille maximale : préfixe (32), deux-points, identifiant (95).
pub const OWNER_REF_MAX: usize = 128;

/// Propriétaire au format vérifié (voir [`is_owner_ref`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OwnerRef(String);

impl OwnerRef {
    /// `None` si le format n'est pas celui d'un propriétaire.
    pub fn parse(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        is_owner_ref(&value).then_some(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for OwnerRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// `<extension>:<identifiant>` : préfixe au format d'un nom d'extension (`[a-z][a-z0-9_]{0,31}`),
/// identifiant de 1 à 95 caractères sûrs dans une URL (`[A-Za-z0-9._~-]`).
pub fn is_owner_ref(value: &str) -> bool {
    let Some((prefix, id)) = value.split_once(':') else {
        return false;
    };
    let prefix_ok = (1..=32).contains(&prefix.len())
        && prefix.starts_with(|c: char| c.is_ascii_lowercase())
        && prefix
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    let id_ok = (1..=95).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '~' | '-'));
    prefix_ok && id_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_extension_prefixed_identifiers() {
        for value in [
            "cloud:6f1c2a9e-3b7d-4c8e-9a51-2d4f8b0c7e13",
            "a:1",
            "my_ext:cus_8f2k1q.v2~x",
        ] {
            assert!(is_owner_ref(value), "{value}");
        }
    }

    #[test]
    fn refuses_anything_else() {
        let long_id = format!("cloud:{}", "a".repeat(96));
        let long_prefix = format!("{}:x", "a".repeat(33));
        for value in [
            "",
            "cloud",
            "cloud:",
            ":abc",
            "Cloud:abc",
            "1cloud:abc",
            "cloud:a b",
            "cloud:a/b",
            "cloud:a:b",
            "cloud:é",
            long_id.as_str(),
            long_prefix.as_str(),
        ] {
            assert!(!is_owner_ref(value), "{value}");
        }
    }

    #[test]
    fn parsing_keeps_only_valid_references() {
        assert_eq!(
            OwnerRef::parse("cloud:abc").map(|o| o.to_string()),
            Some("cloud:abc".to_owned())
        );
        assert!(OwnerRef::parse("abc").is_none());
    }

    #[test]
    fn the_longest_valid_reference_fits_the_column() {
        let value = format!("{}:{}", "a".repeat(32), "b".repeat(95));
        assert!(is_owner_ref(&value));
        assert_eq!(value.len(), OWNER_REF_MAX);
    }
}
