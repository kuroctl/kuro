//! Règles du DNS (ADR-0026) : noms valides, label proposé, enregistrements voulus pour un nœud
//! et pour un serveur ; domaine et identifiants des sites web (ADR-0038). Sans base ni réseau.

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
    /// Domaine des sites web (`web.kuroctl.com`, ADR-0038) ; absent : aucun site publié.
    pub sites: Option<String>,
}

impl ZoneNames {
    pub fn node_host(&self, node: &str) -> String {
        format!("{node}.{}.{}", self.nodes_label, self.base)
    }

    pub fn server_host(&self, label: &str) -> String {
        format!("{label}.{}", self.base)
    }

    /// `<identifiant>.<domaine des sites>`, quand la zone a un domaine des sites.
    pub fn site_host(&self, web_id: &str) -> Option<String> {
        self.sites.as_ref().map(|sites| format!("{web_id}.{sites}"))
    }
}

/// Premières étiquettes interdites au domaine des sites : elles désignent le panel, l'API, la
/// documentation ou des services classiques de la zone.
pub const RESERVED_SITE_LABELS: &[&str] = &["app", "api", "get", "docs", "www", "mail"];

/// Domaine des sites web (ADR-0038) : un domaine valide d'au moins trois étiquettes (jamais la
/// racine de la zone), à l'écart du domaine des jeux et du sous-domaine des nœuds (ni l'un
/// d'eux, ni au-dessus, ni au-dessous : un site ne doit pas pouvoir se faire passer pour un
/// serveur ou un nœud), et dont la première étiquette n'est pas réservée.
pub fn valid_sites_domain(sites: &str, base: &str, nodes_label: &str) -> bool {
    let under = |child: &str, parent: &str| child.ends_with(&format!(".{parent}"));
    let nodes = format!("{nodes_label}.{base}");
    let first = sites.split('.').next().unwrap_or_default();
    valid_domain(sites)
        && sites.split('.').count() >= 3
        && !RESERVED_SITE_LABELS.contains(&first)
        && sites != base
        && !under(sites, base)
        && !under(base, sites)
        && sites != nodes
        && !under(sites, &nodes)
}

/// Longueur d'un identifiant de site.
pub const WEB_ID_LEN: usize = 10;
/// Base32 minuscule (RFC 4648), lisible et valide comme étiquette DNS.
const WEB_ID_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";

/// Identifiant de site tiré de `WEB_ID_LEN` octets aléatoires au moins (générateur
/// cryptographique) : 50 bits, ni collision ni énumération en pratique, et l'index unique de
/// `servers.web_id` tranche le reste. 256 est un multiple de 32 : aucun biais.
pub fn web_id_from(random: &[u8]) -> String {
    random
        .iter()
        .take(WEB_ID_LEN)
        .map(|byte| char::from(WEB_ID_ALPHABET[usize::from(*byte) % WEB_ID_ALPHABET.len()]))
        .collect()
}

pub fn valid_web_id(web_id: &str) -> bool {
    web_id.len() == WEB_ID_LEN && web_id.bytes().all(|b| WEB_ID_ALPHABET.contains(&b))
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

/// `CNAME` d'un serveur vers son nœud et un `SRV` par port qui en déclare un, quand il a un
/// label (un serveur sans port public, un site web, n'en a pas : rien à joindre par ce nom) ;
/// et le `CNAME` de son site (`<identifiant>.<domaine des sites>`, ADR-0038) s'il en a un et que
/// la zone a un domaine des sites. La cible d'un SRV est le nom du nœud (jamais un alias,
/// RFC 2782).
pub fn server_records(
    zone: &ZoneNames,
    label: Option<&str>,
    node: &str,
    srv: &[SrvPort],
    web_id: Option<&str>,
) -> Vec<Desired> {
    if !valid_label(node) || label.is_some_and(|label| !valid_label(label)) {
        return Vec::new();
    }
    let target = zone.node_host(node);
    let mut records = Vec::new();
    if let Some(label) = label {
        let host = zone.server_host(label);
        records.push(Desired {
            kind: RecordKind::Cname,
            name: host.clone(),
            content: target.clone(),
        });
        for entry in srv {
            records.push(Desired {
                kind: RecordKind::Srv,
                name: format!("{}.{host}", entry.service),
                content: format!("0 0 {} {target}", entry.port),
            });
        }
    }
    if let Some(site) = web_id
        .filter(|id| valid_web_id(id))
        .and_then(|id| zone.site_host(id))
    {
        records.push(Desired {
            kind: RecordKind::Cname,
            name: site,
            content: target,
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
            sites: None,
        }
    }

    #[test]
    fn a_web_server_gets_the_cname_of_its_site_when_the_zone_has_a_sites_domain() {
        let without = server_records(&zone(), Some("blog"), "node-1", &[], Some("abcdefg234"));
        assert_eq!(without.len(), 1, "no sites domain: no site record");

        let mut with_sites = zone();
        with_sites.sites = Some("web.kuroctl.com".to_owned());
        let records = server_records(&with_sites, Some("blog"), "node-1", &[], Some("abcdefg234"));
        assert_eq!(
            records,
            vec![
                Desired {
                    kind: RecordKind::Cname,
                    name: "blog.play.kuroctl.com".to_owned(),
                    content: "node-1.nodes.play.kuroctl.com".to_owned(),
                },
                Desired {
                    kind: RecordKind::Cname,
                    name: "abcdefg234.web.kuroctl.com".to_owned(),
                    content: "node-1.nodes.play.kuroctl.com".to_owned(),
                },
            ]
        );
        // Un site web sans port public n'a pas de label : seul le CNAME du site.
        let site_only = server_records(&with_sites, None, "node-1", &[], Some("abcdefg234"));
        assert_eq!(site_only.len(), 1);
        assert_eq!(site_only[0].name, "abcdefg234.web.kuroctl.com");
        // Un serveur de jeu n'a pas de site ; un identifiant forgé n'est pas publié.
        assert_eq!(
            server_records(&with_sites, Some("blog"), "node-1", &[], None).len(),
            1
        );
        assert_eq!(
            server_records(
                &with_sites,
                Some("blog"),
                "node-1",
                &[],
                Some("evil.example")
            )
            .len(),
            1
        );
    }

    #[test]
    fn the_sites_domain_stays_apart_from_servers_nodes_and_reserved_names() {
        let ok = |sites: &str| valid_sites_domain(sites, "play.kuroctl.com", "nodes");
        assert!(ok("web.kuroctl.com"));
        assert!(ok("sites.hosting.kuroctl.com"));
        // La racine de la zone, les noms réservés.
        assert!(!ok("kuroctl.com"));
        assert!(!ok("app.kuroctl.com"));
        assert!(!ok("www.kuroctl.com"));
        assert!(!ok("API.kuroctl.com".to_ascii_lowercase().as_str()));
        // Le domaine des jeux, au-dessus, au-dessous ; le sous-domaine des nœuds.
        assert!(!ok("play.kuroctl.com"));
        assert!(!ok("web.play.kuroctl.com"));
        assert!(!valid_sites_domain(
            "kuroctl.com",
            "eu.play.kuroctl.com",
            "nodes"
        ));
        assert!(!valid_sites_domain(
            "play.kuroctl.com",
            "eu.play.kuroctl.com",
            "nodes"
        ));
        assert!(!ok("nodes.play.kuroctl.com"));
        assert!(!ok("x.nodes.play.kuroctl.com"));
        // Pas un domaine.
        assert!(!ok("web_sites.kuroctl.com"));
        assert!(!ok("web.kuroctl.com."));
    }

    #[test]
    fn a_web_id_is_ten_base32_characters() {
        let id = web_id_from(&[0, 1, 25, 26, 31, 32, 255, 7, 8, 9, 10, 11]);
        assert_eq!(
            id, "abz27a7hij",
            "ten characters, the extra bytes are ignored"
        );
        assert_eq!(id.len(), WEB_ID_LEN);
        assert!(valid_web_id(&id));
        assert!(valid_label(&id), "usable as a DNS label");
        assert!(!valid_web_id("abc"));
        assert!(!valid_web_id("abc123defG"));
        assert!(!valid_web_id("abc123def1"), "0, 1, 8 and 9 are not base32");
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
            Some("survie"),
            "dev-1",
            &[SrvPort {
                service: "_minecraft._tcp".to_owned(),
                port: 25568,
            }],
            None,
        );
        assert_eq!(records[0].kind, RecordKind::Cname);
        assert_eq!(records[0].name, "survie.play.kuroctl.com");
        assert_eq!(records[0].content, "dev-1.nodes.play.kuroctl.com");
        assert_eq!(records[1].name, "_minecraft._tcp.survie.play.kuroctl.com");
        assert_eq!(records[1].content, "0 0 25568 dev-1.nodes.play.kuroctl.com");
    }
}
