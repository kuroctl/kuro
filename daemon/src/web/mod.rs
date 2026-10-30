//! Proxy de bord du nœud (ADR-0038) : un conteneur Caddy, lancé dès qu'un serveur du nœud a un
//! site web, qui sert chaque site en HTTPS et le relaie vers son conteneur sur le réseau privé
//! du serveur. Aucun port de site n'est publié sur l'hôte ; seul le proxy relie les réseaux.
//!
//! Le daemon écrit la configuration complète en JSON typé ([`caddy`]) dans un dossier monté en
//! lecture seule, puis la fait recharger par une exécution dans le conteneur (`caddy reload`) :
//! rechargement entier et idempotent, API d'administration jamais publiée ni montée. Elle est
//! recalculée à chaque déploiement ou suppression d'un serveur web, et au démarrage du daemon.
//!
//! L'état du proxy (prêt, indisponible et pourquoi) et de chaque certificat (en attente, obtenu)
//! accompagne les `server.state` des serveurs web : rien n'échoue en silence.

pub mod caddy;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use kuro_protocol::api::server::WebRoute;
use kuro_protocol::daemon::server::{CertificateState, ProxyState, WebStatus};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::docker::{Docker, DockerError, ProxyContainer, container_name, network_name};

/// Image officielle de Caddy, épinglée par empreinte (2.10.2, Alpine).
pub const DEFAULT_IMAGE: &str = "docker.io/library/caddy@sha256:4c6e91c6ed0e2fa03efd5b44747b625fec79bc9cd06ac5235a779726618e530d";
/// Ports d'écoute dans le conteneur : non privilégiés, Caddy ne tourne pas en root.
const INNER_HTTP_PORT: u16 = 8080;
const INNER_HTTPS_PORT: u16 = 8443;
/// Utilisateur du conteneur (`nonroot` des images distroless, sans entrée `passwd` requise).
const USER: (u32, u32) = (65532, 65532);
const MEMORY_BYTES: u64 = 256 * 1024 * 1024;
const PIDS: u32 = 256;
/// Le rechargement suit de près la création du conteneur : Caddy n'écoute pas encore.
const RELOAD_ATTEMPTS: u32 = 10;
const RELOAD_RETRY: Duration = Duration::from_millis(500);
/// Dossier du proxy sous la racine des données : jamais un identifiant de serveur.
const DIR: &str = "_proxy";
const CONFIG_FILE: &str = "caddy.json";
/// Étiquette qui résume ce qui demande de recréer le conteneur (image, ports).
const LABEL_SPEC: &str = "kuro.proxy_spec";

/// Réglages du proxy (`[web]` de la configuration du daemon).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebConfig {
    /// Ports publiés sur l'hôte. Autres que 80 et 443 pour les tests, un poste de développement,
    /// ou une machine qui porte aussi le Caddy de l'API.
    #[serde(default = "default_http_port")]
    pub http_port: u16,
    #[serde(default = "default_https_port")]
    pub https_port: u16,
    /// Image de Caddy, épinglée par empreinte.
    #[serde(default = "default_image")]
    pub image: String,
    /// `acme` (Let's Encrypt) en production ; `internal` pour les tests et le développement.
    #[serde(default)]
    pub tls: TlsMode,
    #[serde(default)]
    pub acme_email: Option<String>,
    /// Taille maximale du corps d'une requête, en octets (512 Mio par défaut).
    #[serde(default = "default_max_body_bytes")]
    pub max_body_bytes: u64,
    /// Nom du conteneur ; distinct dans les tests pour ne pas toucher au proxy du nœud.
    #[serde(default = "default_container")]
    pub container: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TlsMode {
    #[default]
    Acme,
    Internal,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            http_port: default_http_port(),
            https_port: default_https_port(),
            image: default_image(),
            tls: TlsMode::default(),
            acme_email: None,
            max_body_bytes: default_max_body_bytes(),
            container: default_container(),
        }
    }
}

fn default_http_port() -> u16 {
    80
}

fn default_https_port() -> u16 {
    443
}

fn default_image() -> String {
    DEFAULT_IMAGE.to_owned()
}

fn default_max_body_bytes() -> u64 {
    512 * 1024 * 1024
}

fn default_container() -> String {
    "kuro-proxy".to_owned()
}

#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    #[error(transparent)]
    Docker(#[from] DockerError),
    #[error("proxy configuration: {0}")]
    Config(String),
    #[error("caddy reload failed (exit code {code}): {output}")]
    Reload { code: i64, output: String },
}

#[derive(Debug, Default)]
struct State {
    /// Routes par serveur, dans un ordre stable.
    routes: BTreeMap<Uuid, Vec<WebRoute>>,
    status: HashMap<Uuid, WebStatus>,
    /// Certificats constatés sur le disque, par nom d'hôte.
    issued: HashSet<String>,
}

/// Le proxy du nœud. Sans serveur web, aucun conteneur ne tourne.
#[derive(Debug)]
pub struct Proxy {
    docker: Docker,
    dir: PathBuf,
    config: WebConfig,
    state: Mutex<State>,
    /// Une application à la fois : la dernière configuration écrite est celle rechargée.
    applying: tokio::sync::Mutex<()>,
}

impl Proxy {
    pub fn new(docker: Docker, data_root: &Path, config: WebConfig) -> Self {
        Self {
            docker,
            dir: data_root.join(DIR),
            config,
            state: Mutex::new(State::default()),
            applying: tokio::sync::Mutex::new(()),
        }
    }

    /// État web d'un serveur : présent pour un serveur qui a des routes, absent sinon.
    pub fn status(&self, server_id: Uuid) -> Option<WebStatus> {
        self.state().status.get(&server_id).cloned()
    }

    /// Routes d'un serveur déployé (vides : plus de site), puis configuration appliquée. Un échec
    /// est porté par l'état web de chaque serveur ; il n'arrête pas le déploiement.
    pub async fn set_routes(&self, server_id: Uuid, routes: Vec<WebRoute>) {
        {
            let mut state = self.state();
            if routes.is_empty() {
                state.routes.remove(&server_id);
                state.status.remove(&server_id);
            } else {
                state.routes.insert(server_id, routes);
            }
        }
        self.apply().await;
    }

    /// Serveur supprimé : ses routes disparaissent et le proxy quitte son réseau, avant que le
    /// réseau soit supprimé.
    pub async fn remove(&self, server_id: Uuid) {
        self.set_routes(server_id, Vec::new()).await;
    }

    /// Au démarrage du daemon : les routes de tous les serveurs déployés, d'un coup.
    pub async fn restore(&self, all: Vec<(Uuid, Vec<WebRoute>)>) {
        {
            let mut state = self.state();
            state.routes.clear();
            state.status.clear();
            for (server_id, routes) in all {
                if !routes.is_empty() {
                    state.routes.insert(server_id, routes);
                }
            }
        }
        if !self.state().routes.is_empty() {
            self.apply().await;
        }
    }

    /// Relit les certificats présents sur le disque ; les serveurs dont l'état change.
    pub async fn refresh_certificates(&self) -> Vec<Uuid> {
        let hosts: Vec<String> = self
            .state()
            .routes
            .values()
            .flatten()
            .map(|route| route.host.clone())
            .collect();
        if hosts.is_empty() {
            return Vec::new();
        }
        let data = self.dir.join("data");
        let issued = tokio::task::spawn_blocking(move || issued_hosts(&data, &hosts))
            .await
            .unwrap_or_default();
        let mut state = self.state();
        state.issued = issued;
        let mut changed = Vec::new();
        let servers: Vec<Uuid> = state.routes.keys().copied().collect();
        for server_id in servers {
            let certificate = certificate_state(&state, server_id);
            if let Some(status) = state.status.get_mut(&server_id)
                && status.certificate != certificate
            {
                status.certificate = certificate;
                changed.push(server_id);
            }
        }
        changed
    }

    async fn apply(&self) {
        let _applying = self.applying.lock().await;
        let routes = self.state().routes.clone();
        let outcome = self.apply_routes(&routes).await;
        if let Err(error) = &outcome {
            tracing::warn!(%error, "web proxy not applied");
        }
        let mut state = self.state();
        for server_id in routes.keys() {
            let certificate = certificate_state(&state, *server_id);
            let status = match &outcome {
                Ok(()) => WebStatus {
                    proxy: ProxyState::Ready,
                    certificate,
                    message: None,
                },
                Err(error) => WebStatus {
                    proxy: ProxyState::Unavailable,
                    certificate,
                    message: Some(error.to_string().chars().take(500).collect()),
                },
            };
            state.status.insert(*server_id, status);
        }
    }

    async fn apply_routes(&self, routes: &BTreeMap<Uuid, Vec<WebRoute>>) -> Result<(), ProxyError> {
        let docker = &self.docker;
        let container = self.config.container.as_str();
        if routes.is_empty() {
            // Plus aucun site : le conteneur s'en va, les certificats restent pour un retour.
            docker.remove_container(container).await?;
            return Ok(());
        }
        let sites: Vec<caddy::Site> = routes
            .iter()
            .flat_map(|(server_id, routes)| {
                routes.iter().map(move |route| caddy::Site {
                    host: route.host.clone(),
                    upstream: format!(
                        "{}:{}",
                        container_name(*server_id, &route.service),
                        route.container_port
                    ),
                })
            })
            .collect();
        let options = caddy::Options {
            http_port: INNER_HTTP_PORT,
            https_port: INNER_HTTPS_PORT,
            tls: match self.config.tls {
                TlsMode::Acme => caddy::Tls::Acme,
                TlsMode::Internal => caddy::Tls::Internal,
            },
            acme_email: self.config.acme_email.clone(),
            max_body_bytes: self.config.max_body_bytes,
        };
        let json = serde_json::to_vec_pretty(&caddy::render(&options, &sites))
            .map_err(|e| ProxyError::Config(e.to_string()))?;

        let dir = self.dir.clone();
        let created_dirs = tokio::task::spawn_blocking(move || write_config(&dir, &json))
            .await
            .map_err(|e| ProxyError::Config(e.to_string()))?
            .map_err(|e| ProxyError::Config(e.to_string()))?;
        if created_dirs {
            // Dossiers de Caddy (certificats, configuration persistée) à son utilisateur.
            docker
                .chown(
                    &[
                        (self.dir.join("data"), vec![".".to_owned()]),
                        (self.dir.join("home"), vec![".".to_owned()]),
                    ],
                    USER.0,
                    USER.1,
                )
                .await?;
        }

        docker.refresh_image(&self.config.image).await?;
        let spec_label = format!(
            "{}|{}|{}",
            self.config.image, self.config.http_port, self.config.https_port
        );
        let existing = docker.labels_of(container).await?;
        let mut created = false;
        match existing {
            Some(labels) if labels.get(LABEL_SPEC) == Some(&spec_label) => {}
            Some(_) => {
                docker.remove_container(container).await?;
                self.create(container, &spec_label).await?;
                created = true;
            }
            None => {
                self.create(container, &spec_label).await?;
                created = true;
            }
        }

        // Réseaux : celui de chaque serveur web, et aucun autre (hors le pont par défaut).
        let wanted: HashSet<String> = routes.keys().map(|id| network_name(*id)).collect();
        let current: HashSet<String> = docker.networks_of(container).await?.into_iter().collect();
        for network in wanted.difference(&current) {
            docker.connect_network(container, network).await?;
        }
        for network in current.difference(&wanted) {
            if network.starts_with("kuro-") {
                docker.disconnect_network(container, network).await?;
            }
        }

        if !docker.status(container).await?.is_some_and(|s| s.running) {
            docker.start(container).await?;
            created = true;
        }
        if created {
            // Un conteneur neuf charge la configuration au démarrage : rien à recharger, mais
            // l'administration doit répondre avant le prochain rechargement.
            self.wait_admin(container).await?;
        } else {
            self.reload(container).await?;
        }
        Ok(())
    }

    async fn create(&self, container: &str, spec_label: &str) -> Result<(), ProxyError> {
        let spec = ProxyContainer {
            name: container,
            image: &self.config.image,
            command: vec![
                "caddy".to_owned(),
                "run".to_owned(),
                "--config".to_owned(),
                format!("/etc/caddy/{CONFIG_FILE}"),
            ],
            env: vec![
                "XDG_DATA_HOME=/data".to_owned(),
                "XDG_CONFIG_HOME=/config".to_owned(),
            ],
            user: &format!("{}:{}", USER.0, USER.1),
            mounts: vec![
                ("/etc/caddy".to_owned(), self.dir.join("config"), true),
                ("/data".to_owned(), self.dir.join("data"), false),
                ("/config".to_owned(), self.dir.join("home"), false),
            ],
            ports: vec![
                (INNER_HTTP_PORT, self.config.http_port),
                (INNER_HTTPS_PORT, self.config.https_port),
            ],
            labels: HashMap::from([(LABEL_SPEC.to_owned(), spec_label.to_owned())]),
            memory_bytes: MEMORY_BYTES,
            pids: PIDS,
        };
        self.docker.create_proxy(&spec).await?;
        Ok(())
    }

    async fn reload(&self, container: &str) -> Result<(), ProxyError> {
        let command = vec![
            "caddy".to_owned(),
            "reload".to_owned(),
            "--config".to_owned(),
            format!("/etc/caddy/{CONFIG_FILE}"),
        ];
        let mut last = (0, String::new());
        for attempt in 0..RELOAD_ATTEMPTS {
            if attempt > 0 {
                tokio::time::sleep(RELOAD_RETRY).await;
            }
            let (code, output) = self.docker.exec(container, command.clone()).await?;
            if code == 0 {
                return Ok(());
            }
            last = (code, output);
        }
        Err(ProxyError::Reload {
            code: last.0,
            output: last.1.chars().take(400).collect(),
        })
    }

    /// Caddy démarré : son administration répond (`caddy reload` de la même configuration).
    async fn wait_admin(&self, container: &str) -> Result<(), ProxyError> {
        self.reload(container).await
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn certificate_state(state: &State, server_id: Uuid) -> CertificateState {
    let all_issued = state.routes.get(&server_id).is_some_and(|routes| {
        routes
            .iter()
            .all(|route| state.issued.contains(&route.host))
    });
    if all_issued {
        CertificateState::Issued
    } else {
        CertificateState::Pending
    }
}

/// Écrit la configuration (fichier temporaire puis renommage : jamais lue à moitié). `true` si
/// les dossiers viennent d'être créés.
fn write_config(dir: &Path, json: &[u8]) -> std::io::Result<bool> {
    let created = !dir.join("data").exists();
    for sub in ["config", "data", "home"] {
        std::fs::create_dir_all(dir.join(sub))?;
    }
    let target = dir.join("config").join(CONFIG_FILE);
    let temporary = dir.join("config").join(format!("{CONFIG_FILE}.tmp"));
    std::fs::write(&temporary, json)?;
    std::fs::rename(&temporary, &target)?;
    Ok(created)
}

/// Noms dont Caddy a obtenu un certificat : `<data>/caddy/certificates/<émetteur>/<nom>/<nom>.crt`.
fn issued_hosts(data: &Path, hosts: &[String]) -> HashSet<String> {
    let Ok(issuers) = std::fs::read_dir(data.join("caddy").join("certificates")) else {
        return HashSet::new();
    };
    let mut issued = HashSet::new();
    for issuer in issuers.filter_map(Result::ok) {
        for host in hosts {
            if issuer
                .path()
                .join(host)
                .join(format!("{host}.crt"))
                .is_file()
            {
                issued.insert(host.clone());
            }
        }
    }
    issued
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_image_is_pinned_by_digest_on_the_standard_ports() {
        let config = WebConfig::default();
        assert!(config.image.contains("@sha256:"));
        assert_eq!((config.http_port, config.https_port), (80, 443));
        assert_eq!(config.tls, TlsMode::Acme);
    }

    #[test]
    fn certificates_are_found_where_caddy_stores_them() {
        let dir = tempfile::tempdir().unwrap();
        let host = "abcdefg234.web.kuroctl.test";
        let cert = dir
            .path()
            .join("caddy/certificates/local")
            .join(host)
            .join(format!("{host}.crt"));
        std::fs::create_dir_all(cert.parent().unwrap()).unwrap();
        std::fs::write(&cert, b"cert").unwrap();

        let issued = issued_hosts(dir.path(), &[host.to_owned(), "other.example".to_owned()]);

        assert_eq!(issued, HashSet::from([host.to_owned()]));
        assert!(issued_hosts(&dir.path().join("missing"), &[host.to_owned()]).is_empty());
    }

    #[test]
    fn the_configuration_is_written_whole_and_the_data_dirs_once() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("_proxy");

        assert!(
            write_config(&root, b"{}").unwrap(),
            "first write creates the dirs"
        );
        assert!(!write_config(&root, b"{\"a\":1}").unwrap());
        assert_eq!(
            std::fs::read_to_string(root.join("config/caddy.json")).unwrap(),
            "{\"a\":1}"
        );
        assert!(!root.join("config/caddy.json.tmp").exists());
        assert!(root.join("data").is_dir() && root.join("home").is_dir());
    }
}
