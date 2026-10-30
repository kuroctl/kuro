//! Configuration JSON de Caddy, typée (ADR-0038) : des structures `serde`, jamais un Caddyfile
//! assemblé par concaténation. Un nom d'hôte ou un conteneur ne peut pas s'échapper de son champ.
//!
//! Un seul serveur HTTP, qui écoute sur le port HTTPS du conteneur ; Caddy ajoute lui-même la
//! redirection HTTP → HTTPS et répond aux défis ACME sur le port HTTP. Chaque site est une route
//! par nom d'hôte vers son conteneur, sur le réseau privé de son serveur.

use serde::Serialize;

/// Un site servi : nom d'hôte public et conteneur à joindre (`kuro-<id>-web:80`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Site {
    pub host: String,
    pub upstream: String,
}

/// Comment les certificats sont obtenus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tls {
    /// Let's Encrypt, défis HTTP-01 et TLS-ALPN sur le nœud.
    Acme,
    /// Autorité interne de Caddy : tests et développement, jamais reconnue par un navigateur.
    Internal,
}

#[derive(Debug, Clone)]
pub struct Options {
    /// Ports d'écoute dans le conteneur (non privilégiés : Caddy ne tourne pas en root).
    pub http_port: u16,
    pub https_port: u16,
    pub tls: Tls,
    /// Adresse de contact donnée à Let's Encrypt (expiration, incidents).
    pub acme_email: Option<String>,
    /// Taille maximale du corps d'une requête (médias d'un site).
    pub max_body_bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct Config {
    admin: Admin,
    apps: Apps,
}

#[derive(Debug, Serialize)]
struct Admin {
    /// Dans le conteneur seulement : `caddy reload` y parle, rien n'est publié ni monté.
    listen: &'static str,
}

#[derive(Debug, Serialize)]
struct Apps {
    http: HttpApp,
    tls: TlsApp,
}

#[derive(Debug, Serialize)]
struct HttpApp {
    http_port: u16,
    https_port: u16,
    servers: Servers,
}

#[derive(Debug, Serialize)]
struct Servers {
    sites: Server,
}

#[derive(Debug, Serialize)]
struct Server {
    listen: Vec<String>,
    routes: Vec<Route>,
}

#[derive(Debug, Serialize)]
struct Route {
    #[serde(rename = "match")]
    matchers: Vec<Matcher>,
    handle: Vec<Handler>,
    terminal: bool,
}

#[derive(Debug, Serialize)]
struct Matcher {
    host: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "handler", rename_all = "snake_case")]
enum Handler {
    RequestBody { max_size: u64 },
    ReverseProxy { upstreams: Vec<Upstream> },
}

#[derive(Debug, Serialize)]
struct Upstream {
    dial: String,
}

#[derive(Debug, Serialize)]
struct TlsApp {
    automation: Automation,
}

#[derive(Debug, Serialize)]
struct Automation {
    policies: Vec<Policy>,
}

#[derive(Debug, Serialize)]
struct Policy {
    /// Les noms servis, et eux seuls : aucun certificat n'est demandé pour un autre nom.
    subjects: Vec<String>,
    issuers: Vec<Issuer>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "module", rename_all = "snake_case")]
enum Issuer {
    Acme {
        #[serde(skip_serializing_if = "Option::is_none")]
        email: Option<String>,
    },
    Internal,
}

/// Configuration complète pour `sites` (dans l'ordre reçu, déterministe).
pub fn render(options: &Options, sites: &[Site]) -> Config {
    let routes = sites
        .iter()
        .map(|site| Route {
            matchers: vec![Matcher {
                host: vec![site.host.clone()],
            }],
            handle: vec![
                Handler::RequestBody {
                    max_size: options.max_body_bytes,
                },
                Handler::ReverseProxy {
                    upstreams: vec![Upstream {
                        dial: site.upstream.clone(),
                    }],
                },
            ],
            terminal: true,
        })
        .collect();
    let issuer = match options.tls {
        Tls::Acme => Issuer::Acme {
            email: options.acme_email.clone(),
        },
        Tls::Internal => Issuer::Internal,
    };
    Config {
        admin: Admin {
            listen: "localhost:2019",
        },
        apps: Apps {
            http: HttpApp {
                http_port: options.http_port,
                https_port: options.https_port,
                servers: Servers {
                    sites: Server {
                        listen: vec![format!(":{}", options.https_port)],
                        routes,
                    },
                },
            },
            tls: TlsApp {
                automation: Automation {
                    policies: vec![Policy {
                        subjects: sites.iter().map(|site| site.host.clone()).collect(),
                        issuers: vec![issuer],
                    }],
                },
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn options(tls: Tls) -> Options {
        Options {
            http_port: 8080,
            https_port: 8443,
            tls,
            acme_email: Some("ops@example.test".to_owned()),
            max_body_bytes: 512 * 1024 * 1024,
        }
    }

    #[test]
    fn each_site_is_a_terminal_route_to_its_container() {
        let sites = [
            Site {
                host: "abcdefg234.web.kuroctl.test".to_owned(),
                upstream: "kuro-0192f3a4-1111-7000-8000-00000000abcd-web:80".to_owned(),
            },
            Site {
                host: "zzzzzzz777.web.kuroctl.test".to_owned(),
                upstream: "kuro-0192f3a4-2222-7000-8000-00000000abcd-web:8080".to_owned(),
            },
        ];

        let config = serde_json::to_value(render(&options(Tls::Acme), &sites)).unwrap();

        assert_eq!(config["admin"]["listen"], "localhost:2019");
        assert_eq!(config["apps"]["http"]["http_port"], 8080);
        assert_eq!(config["apps"]["http"]["https_port"], 8443);
        let server = &config["apps"]["http"]["servers"]["sites"];
        assert_eq!(server["listen"], json!([":8443"]));
        assert_eq!(
            server["routes"][1],
            json!({
                "match": [{ "host": ["zzzzzzz777.web.kuroctl.test"] }],
                "handle": [
                    { "handler": "request_body", "max_size": 536_870_912 },
                    { "handler": "reverse_proxy",
                      "upstreams": [{ "dial": "kuro-0192f3a4-2222-7000-8000-00000000abcd-web:8080" }] }
                ],
                "terminal": true
            })
        );
        assert_eq!(
            config["apps"]["tls"]["automation"]["policies"],
            json!([{
                "subjects": ["abcdefg234.web.kuroctl.test", "zzzzzzz777.web.kuroctl.test"],
                "issuers": [{ "module": "acme", "email": "ops@example.test" }]
            }])
        );
    }

    #[test]
    fn the_internal_issuer_has_no_email_and_a_host_cannot_escape_its_field() {
        let sites = [Site {
            host: "evil\",\"other.example".to_owned(),
            upstream: "kuro-x-web:80".to_owned(),
        }];

        let text = serde_json::to_string(&render(&options(Tls::Internal), &sites)).unwrap();
        let config: serde_json::Value = serde_json::from_str(&text).unwrap();

        assert_eq!(
            config["apps"]["tls"]["automation"]["policies"][0]["issuers"],
            json!([{ "module": "internal" }])
        );
        // Le nom reste une seule chaîne, échappée : aucun second hôte n'apparaît.
        assert_eq!(
            config["apps"]["http"]["servers"]["sites"]["routes"][0]["match"][0]["host"],
            json!(["evil\",\"other.example"])
        );
    }
}
