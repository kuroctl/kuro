//! Règles de validation partagées par plusieurs domaines.

/// Identifiant lisible : `[a-z0-9-]`, sans tiret en bordure (forme d'un label DNS).
pub fn is_slug(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !value.starts_with('-')
        && !value.ends_with('-')
}

/// Texte affiché (nom, description) : borné, sans caractère de contrôle.
pub fn is_display_text(value: &str, min_len: usize, max_len: usize) -> bool {
    let length = value.chars().count();
    (min_len..=max_len).contains(&length) && !value.chars().any(char::is_control)
}

/// Adresse joignable sur Internet : jamais une adresse privée, locale, de lien ou réservée. Toute
/// requête sortante de l'API vers une adresse choisie par un tiers (document OAuth, source de
/// contenu) la vérifie après résolution, contre la falsification de requête côté serveur.
pub fn is_public(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, c, _] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
                || a == 0
                || a >= 240
                || (a == 100 && (64..128).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 198 && (18..20).contains(&b)))
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
                || first == 0x2001 && v6.segments()[1] == 0x0db8
                || first == 0x0064 && v6.segments()[1] == 0xff9b)
        }
    }
}
