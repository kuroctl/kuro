//! Règles du DNS (ADR-0026) : noms valides, label proposé, enregistrements voulus pour un nœud
//! et pour un serveur. Sans base ni réseau.

use std::net::{Ipv4Addr, Ipv6Addr};

/// Nature d'un enregistrement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RecordKind {
    A,
    Aaaa,
    Cname,
    Srv,
}

impl RecordKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::Aaaa => "AAAA",
            Self::Cname => "CNAME",
            Self::Srv => "SRV",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "A" => Some(Self::A),
            "AAAA" => Some(Self::Aaaa),
            "CNAME" => Some(Self::Cname),
            "SRV" => Some(Self::Srv),
            _ => None,
        }
    }
}

/// Enregistrement que Kuro veut voir publié. `content` : adresse, nom cible, ou pour un SRV
/// `priorité poids port cible`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Desired {
    pub kind: RecordKind,
    pub name: String,
    pub content: String,
}

/// Étiquette DNS : minuscules, chiffres et tirets, 1 à 63 caractères, ni tiret au début ni à la
/// fin.
pub fn valid_label(label: &str) -> bool {
    let bytes = label.as_bytes();
    (1..=63).contains(&bytes.len())
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
        && bytes.first() != Some(&b'-')
        && bytes.last() != Some(&b'-')
}

/// Domaine de base (`play.kuroctl.com`) : au moins deux étiquettes valides, 253 caractères au
/// plus.
pub fn valid_domain(domain: &str) -> bool {
    domain.len() <= 253 && domain.split('.').count() >= 2 && domain.split('.').all(valid_label)
}

/// Label proposé depuis le nom d'un serveur : sans accents ni espaces, minuscules.
pub fn suggest_label(name: &str) -> String {
    let mut label = String::new();
    for c in name.chars() {
        let c = match c {
            'à' | 'â' | 'ä' | 'á' | 'À' | 'Â' | 'Ä' | 'Á' => 'a',
            'é' | 'è' | 'ê' | 'ë' | 'É' | 'È' | 'Ê' | 'Ë' => 'e',
            'î' | 'ï' | 'í' | 'Î' | 'Ï' | 'Í' => 'i',
            'ô' | 'ö' | 'ó' | 'Ô' | 'Ö' | 'Ó' => 'o',
            'ù' | 'û' | 'ü' | 'ú' | 'Ù' | 'Û' | 'Ü' | 'Ú' => 'u',
            'ç' | 'Ç' => 'c',
            other => other,
        };
        if c.is_ascii_alphanumeric() {
            label.push(c.to_ascii_lowercase());
        } else if !label.ends_with('-') && !label.is_empty() {
            label.push('-');
        }
    }
    let label: String = label.trim_end_matches('-').chars().take(55).collect();
    let label = label.trim_end_matches('-').to_owned();
    if label.is_empty() {
        "server".to_owned()
    } else {
        label
    }
}

/// Zone vue par le calcul des enregistrements.
#[derive(Debug, Clone)]
pub struct ZoneNames {
    /// `play.kuroctl.com`.
    pub base: String,
    /// `nodes` : les nœuds sont publiés sous `<nœud>.nodes.<base>`.
    pub nodes_label: String,
}

impl ZoneNames {
    pub fn node_host(&self, node: &str) -> String {
        format!("{node}.{}.{}", self.nodes_label, self.base)
    }

    pub fn server_host(&self, label: &str) -> String {
        format!("{label}.{}", self.base)
    }
}

/// `A` et `AAAA` d'un nœud vers son adresse publique.
pub fn node_records(
    zone: &ZoneNames,
    node: &str,
    ipv4: Option<Ipv4Addr>,
    ipv6: Option<Ipv6Addr>,
) -> Vec<Desired> {
    if !valid_label(node) {
        return Vec::new();
    }
    let host = zone.node_host(node);
    let mut records = Vec::new();
    if let Some(ip) = ipv4 {
        records.push(Desired {
            kind: RecordKind::A,
            name: host.clone(),
            content: ip.to_string(),
        });
    }
    if let Some(ip) = ipv6 {
        records.push(Desired {
            kind: RecordKind::Aaaa,
            name: host,
            content: ip.to_string(),
        });
    }
    records
}

/// Port publié en SRV : service déclaré par le prefab et port alloué sur le nœud.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SrvPort {
    pub service: String,
    pub port: u16,
}

/// `CNAME` d'un serveur vers son nœud, et un `SRV` par port qui en déclare un. La cible d'un SRV
/// est le nom du nœud (jamais un alias, RFC 2782).
pub fn server_records(zone: &ZoneNames, label: &str, node: &str, srv: &[SrvPort]) -> Vec<Desired> {
    if !valid_label(label) || !valid_label(node) {
        return Vec::new();
    }
    let host = zone.server_host(label);
    let target = zone.node_host(node);
    let mut records = vec![Desired {
        kind: RecordKind::Cname,
        name: host.clone(),
        content: target.clone(),
    }];
    for entry in srv {
        records.push(Desired {
            kind: RecordKind::Srv,
            name: format!("{}.{host}", entry.service),
            content: format!("0 0 {} {target}", entry.port),
        });
    }
    records
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone() -> ZoneNames {
        ZoneNames {
            base: "play.kuroctl.com".to_owned(),
            nodes_label: "nodes".to_owned(),
        }
    }

    #[test]
    fn labels_follow_dns_rules() {
        assert!(valid_label("survie"));
        assert!(valid_label("dev-1"));
        assert!(!valid_label("-survie"));
        assert!(!valid_label("Survie"));
        assert!(!valid_label("sur_vie"));
        assert!(!valid_label(&"a".repeat(64)));
        assert!(valid_domain("play.kuroctl.com"));
        assert!(!valid_domain("kuroctl"));
        assert!(!valid_domain("play..kuroctl.com"));
    }

    #[test]
    fn a_label_is_suggested_from_the_server_name() {
        assert_eq!(suggest_label("Survie Été"), "survie-ete");
        assert_eq!(suggest_label("  Mon serveur #2 !"), "mon-serveur-2");
        assert_eq!(suggest_label("!!!"), "server");
    }

    #[test]
    fn a_node_gets_a_and_aaaa() {
        let records = node_records(
            &zone(),
            "dev-1",
            Some("203.0.113.7".parse().unwrap()),
            Some("2001:db8::7".parse().unwrap()),
        );
        assert_eq!(
            records,
            [
                Desired {
                    kind: RecordKind::A,
                    name: "dev-1.nodes.play.kuroctl.com".to_owned(),
                    content: "203.0.113.7".to_owned(),
                },
                Desired {
                    kind: RecordKind::Aaaa,
                    name: "dev-1.nodes.play.kuroctl.com".to_owned(),
                    content: "2001:db8::7".to_owned(),
                },
            ]
        );
        assert!(node_records(&zone(), "dev-1", None, None).is_empty());
    }

    #[test]
    fn a_server_points_to_its_node_and_publishes_declared_srv() {
        let records = server_records(
            &zone(),
            "survie",
            "dev-1",
            &[SrvPort {
                service: "_minecraft._tcp".to_owned(),
                port: 25568,
            }],
        );
        assert_eq!(records[0].kind, RecordKind::Cname);
        assert_eq!(records[0].name, "survie.play.kuroctl.com");
        assert_eq!(records[0].content, "dev-1.nodes.play.kuroctl.com");
        assert_eq!(records[1].name, "_minecraft._tcp.survie.play.kuroctl.com");
        assert_eq!(records[1].content, "0 0 25568 dev-1.nodes.play.kuroctl.com");
    }
}
