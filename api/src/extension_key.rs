//! Clé déclarée par une extension (ADR-0051) : permission, action d'audit, code d'erreur.
//! Toujours `<extension>.<segment>(.<segment>)*`, segments `[a-z][a-z0-9_]*` : préfixée par le
//! nom de l'extension, elle ne peut pas rejoindre l'espace de noms de Kuro ni d'une autre.

/// Vérifie qu'une clé est préfixée par `extension` et faite de segments en minuscules.
pub fn is_extension_key(extension: &str, key: &str) -> bool {
    let Some(rest) = key
        .strip_prefix(extension)
        .and_then(|rest| rest.strip_prefix('.'))
    else {
        return false;
    };
    !rest.is_empty()
        && rest.split('.').all(|segment| {
            segment.starts_with(|c: char| c.is_ascii_lowercase())
                && segment
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_carry_the_extension_prefix_and_lowercase_segments() {
        assert!(is_extension_key("cloud", "cloud.catalog.slug_taken"));
        assert!(is_extension_key("cloud", "cloud.x2"));
        for key in [
            "cloud",
            "cloud.",
            "clouds.x",
            "other.x",
            "cloud.X",
            "cloud..x",
            "cloud.1x",
            "cloud.a-b",
        ] {
            assert!(!is_extension_key("cloud", key), "{key}");
        }
    }
}
