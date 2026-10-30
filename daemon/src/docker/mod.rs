//! Adaptateur Docker (`bollard`) : API typée, aucune commande shell construite.
//!
//! Chaque conteneur de serveur reçoit le durcissement de l'ADR-0013, quelle que soit la
//! spécification : utilisateur non-root, racine en lecture seule (`tmpfs` `exec,nosuid,nodev`
//! pour les chemins `writable`), `cap_drop: ALL`, `no-new-privileges`, limites mémoire (sans
//! swap), CPU et PIDs, journaux bornés, un réseau par serveur.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use bollard::models::{
    ContainerCreateBody, ContainerStatsResponse, ContainerUpdateBody, EndpointSettings, ExecConfig,
    HealthConfig, HealthStatusEnum, HostConfig, HostConfigLogConfig, Mount, MountTypeEnum,
    NetworkConnectRequest, NetworkCreateRequest, NetworkDisconnectRequest, NetworkingConfig,
    PortBinding, RestartPolicy, RestartPolicyNameEnum,
};
use bollard::query_parameters::{
    AttachContainerOptionsBuilder, CreateContainerOptionsBuilder, CreateImageOptionsBuilder,
    EventsOptionsBuilder, KillContainerOptionsBuilder, ListContainersOptionsBuilder,
    LogsOptionsBuilder, RemoveContainerOptionsBuilder, StatsOptionsBuilder,
    StopContainerOptionsBuilder, UploadToContainerOptionsBuilder,
};
use futures_util::{Stream, StreamExt};
use kuro_protocol::api::server::{Limits, PortProtocol, ServiceSpec};
use kuro_protocol::limits::MAX_CONSOLE_LINE_BYTES;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

pub const LABEL_MANAGED: &str = "kuro.managed";
pub const LABEL_SERVER: &str = "kuro.server_id";
pub const LABEL_SERVICE: &str = "kuro.service";
const TMPFS_OPTIONS: &str = "rw,exec,nosuid,nodev";

#[derive(Debug, thiserror::Error)]
pub enum DockerError {
    #[error("docker: {0}")]
    Api(#[from] bollard::errors::Error),
    #[error("image pull failed for {image}: {message}")]
    Pull { image: String, message: String },
    #[error("helper container exited with code {0}")]
    Helper(i64),
    #[error("archive: {0}")]
    Archive(#[from] std::io::Error),
}

/// Conteneur du proxy des sites web (ADR-0038) : durci comme un service, mais avec des ports
/// publiés choisis par la configuration, redémarré avec Docker, et relié ensuite aux réseaux des
/// serveurs qu'il sert.
pub struct ProxyContainer<'a> {
    pub name: &'a str,
    pub image: &'a str,
    pub command: Vec<String>,
    pub env: Vec<String>,
    /// `uid:gid`, jamais root.
    pub user: &'a str,
    /// (chemin dans le conteneur, dossier hôte, lecture seule).
    pub mounts: Vec<(String, PathBuf, bool)>,
    /// (port du conteneur, port de l'hôte), TCP, toutes adresses.
    pub ports: Vec<(u16, u16)>,
    pub labels: HashMap<String, String>,
    pub memory_bytes: u64,
    pub pids: u32,
}

/// Fichier déposé dans un conteneur créé, avant son démarrage.
pub struct ContainerFile<'a> {
    /// Dossier du conteneur, qui doit exister dans l'image.
    pub dir: &'a str,
    pub name: &'a str,
    pub content: &'a [u8],
    /// Propriétaire numérique (`uid:gid` de l'utilisateur du conteneur).
    pub owner: (u64, u64),
    pub mode: u32,
}

fn is_not_found(error: &bollard::errors::Error) -> bool {
    matches!(
        error,
        bollard::errors::Error::DockerResponseServerError {
            status_code: 404,
            ..
        }
    )
}

pub fn network_name(server_id: Uuid) -> String {
    format!("kuro-{server_id}")
}

pub fn container_name(server_id: Uuid, service: &str) -> String {
    format!("kuro-{server_id}-{service}")
}

/// Nom de service du conteneur d'installation (`kuro-<id>-kuro-install`).
pub const INSTALLER: &str = "kuro-install";

/// Conteneur d'installation à créer (voir [`Docker::create_installer`]).
#[derive(Debug, Clone)]
pub struct Installer<'a> {
    pub server_id: Uuid,
    pub image: &'a str,
    /// Tableau d'arguments : aucune commande shell construite par Kuro.
    pub command: Vec<String>,
    pub env: Vec<String>,
    pub user: &'a str,
    /// (chemin du conteneur, dossier de l'hôte).
    pub mounts: Vec<(String, PathBuf)>,
    pub memory_bytes: u64,
    pub cpu_millis: u32,
    pub pids: u32,
}

/// Point de départ de la lecture de console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleFrom {
    /// Les N dernières lignes, puis le direct.
    Tail(usize),
    /// Depuis cet instant (secondes Unix), puis le direct.
    Since(i64),
}

/// Ligne de console avec son horodatage Docker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimedLine {
    /// Nanosecondes Unix ; 0 si Docker n'a pas fourni d'horodatage lisible.
    pub at_ns: i128,
    pub text: String,
}

impl TimedLine {
    /// Ligne `docker logs --timestamps` : `<RFC 3339> <texte>`.
    fn parse(line: &str) -> Self {
        let parsed = line.split_once(' ').and_then(|(stamp, text)| {
            let at =
                time::OffsetDateTime::parse(stamp, &time::format_description::well_known::Rfc3339)
                    .ok()?;
            Some(Self {
                at_ns: at.unix_timestamp_nanos(),
                text: text.to_owned(),
            })
        });
        parsed.unwrap_or_else(|| Self {
            at_ns: 0,
            text: line.to_owned(),
        })
    }

    pub fn at_ms(&self) -> i64 {
        i64::try_from(self.at_ns / 1_000_000).unwrap_or(0)
    }
}

/// État d'un conteneur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContainerStatus {
    pub running: bool,
    pub exit_code: Option<i64>,
    pub healthy: Option<bool>,
}

/// Événement Docker d'un conteneur Kuro.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerEvent {
    pub server_id: Uuid,
    pub service: String,
    pub action: String,
    pub exit_code: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct Docker {
    client: bollard::Docker,
    helper_image: String,
}

impl Docker {
    pub fn connect(helper_image: &str) -> Result<Self, DockerError> {
        Ok(Self {
            client: bollard::Docker::connect_with_defaults()?,
            helper_image: helper_image.to_owned(),
        })
    }

    /// Docker joint en HTTP (`http://hôte:port`), avec un délai par requête, plutôt que par la
    /// socket locale : pour un Docker distant, ou un faux Docker dans les tests du lien.
    pub fn connect_http(
        addr: &str,
        timeout: Duration,
        helper_image: &str,
    ) -> Result<Self, DockerError> {
        Ok(Self {
            client: bollard::Docker::connect_with_http(
                addr,
                timeout.as_secs(),
                bollard::API_DEFAULT_VERSION,
            )?,
            helper_image: helper_image.to_owned(),
        })
    }

    pub async fn ping(&self) -> Result<(), DockerError> {
        self.client.ping().await?;
        Ok(())
    }

    /// Télécharge l'image si elle est absente.
    pub async fn ensure_image(&self, image: &str) -> Result<(), DockerError> {
        match self.client.inspect_image(image).await {
            Ok(_) => return Ok(()),
            Err(error) if !is_not_found(&error) => return Err(error.into()),
            Err(_) => {}
        }
        self.pull(image).await
    }

    /// Image à jour : un tag est retiré (les correctifs de l'image de base suivent), une image
    /// épinglée par condensat n'est téléchargée que si elle manque. Registre injoignable : la
    /// copie locale sert, et c'est journalisé.
    pub async fn refresh_image(&self, image: &str) -> Result<(), DockerError> {
        if image.contains("@sha256:") {
            return self.ensure_image(image).await;
        }
        match self.pull(image).await {
            Ok(()) => Ok(()),
            Err(error) => match self.client.inspect_image(image).await {
                Ok(_) => {
                    tracing::warn!(image, %error, "image not refreshed, using the local copy");
                    Ok(())
                }
                Err(_) => Err(error),
            },
        }
    }

    async fn pull(&self, image: &str) -> Result<(), DockerError> {
        let options = CreateImageOptionsBuilder::new().from_image(image).build();
        let mut progress = self.client.create_image(Some(options), None, None);
        while let Some(event) = progress.next().await {
            let event = event.map_err(|error| DockerError::Pull {
                image: image.to_owned(),
                message: error.to_string(),
            })?;
            if let Some(detail) = event.error_detail.and_then(|d| d.message) {
                return Err(DockerError::Pull {
                    image: image.to_owned(),
                    message: detail,
                });
            }
        }
        Ok(())
    }

    /// Réseau bridge propre au serveur : ses services s'y joignent par leur nom.
    pub async fn ensure_network(&self, server_id: Uuid) -> Result<(), DockerError> {
        let name = network_name(server_id);
        match self.client.inspect_network(&name, None).await {
            Ok(_) => return Ok(()),
            Err(error) if !is_not_found(&error) => return Err(error.into()),
            Err(_) => {}
        }
        self.client
            .create_network(NetworkCreateRequest {
                name,
                driver: Some("bridge".to_owned()),
                labels: Some(labels(server_id, None)),
                ..Default::default()
            })
            .await?;
        Ok(())
    }

    pub async fn remove_network(&self, server_id: Uuid) -> Result<(), DockerError> {
        match self.client.remove_network(&network_name(server_id)).await {
            Err(error) if !is_not_found(&error) => Err(error.into()),
            _ => Ok(()),
        }
    }

    /// (Re)crée le conteneur d'un service. `mounts` : (chemin dans le conteneur, dossier hôte).
    pub async fn create_service(
        &self,
        server_id: Uuid,
        service: &ServiceSpec,
        mounts: &[(String, PathBuf)],
    ) -> Result<(), DockerError> {
        let name = container_name(server_id, &service.name);
        self.remove_container(&name).await?;

        let mut exposed = Vec::new();
        let mut bindings: HashMap<String, Option<Vec<PortBinding>>> = HashMap::new();
        for port in &service.ports {
            let key = format!(
                "{}/{}",
                port.container_port,
                match port.protocol {
                    PortProtocol::Tcp => "tcp",
                    PortProtocol::Udp => "udp",
                }
            );
            exposed.push(key.clone());
            if let Some(published) = &port.published {
                bindings.insert(
                    key,
                    Some(vec![PortBinding {
                        host_ip: Some(published.host_ip.clone()),
                        host_port: Some(published.host_port.to_string()),
                    }]),
                );
            }
        }

        let network = network_name(server_id);
        let host_config = HostConfig {
            mounts: Some(
                mounts
                    .iter()
                    .map(|(target, source)| Mount {
                        target: Some(target.clone()),
                        source: Some(source.to_string_lossy().into_owned()),
                        typ: Some(MountTypeEnum::BIND),
                        read_only: Some(false),
                        ..Default::default()
                    })
                    .collect(),
            ),
            tmpfs: Some(
                service
                    .writable
                    .iter()
                    .map(|path| (path.clone(), TMPFS_OPTIONS.to_owned()))
                    .collect(),
            ),
            readonly_rootfs: Some(true),
            cap_drop: Some(vec!["ALL".to_owned()]),
            security_opt: Some(vec!["no-new-privileges".to_owned()]),
            memory: i64::try_from(service.limits.memory_bytes).ok(),
            memory_swap: i64::try_from(service.limits.memory_bytes).ok(),
            nano_cpus: Some(i64::from(service.limits.cpu_millis) * 1_000_000),
            pids_limit: Some(i64::from(service.limits.pids)),
            port_bindings: Some(bindings),
            network_mode: Some(network.clone()),
            log_config: Some(HostConfigLogConfig {
                typ: Some("json-file".to_owned()),
                config: Some(HashMap::from([
                    ("max-size".to_owned(), "10m".to_owned()),
                    ("max-file".to_owned(), "3".to_owned()),
                ])),
            }),
            init: Some(false),
            ..Default::default()
        };

        let body = ContainerCreateBody {
            image: Some(service.image.clone()),
            cmd: (!service.command.is_empty()).then(|| service.command.clone()),
            env: Some(
                service
                    .env
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect(),
            ),
            user: Some(service.user.clone()),
            labels: Some(labels(server_id, Some(&service.name))),
            exposed_ports: Some(exposed),
            // Entrée de la console : commandes et arrêt par commande (`stop`).
            open_stdin: Some(true),
            attach_stdin: Some(false),
            stdin_once: Some(false),
            tty: Some(false),
            healthcheck: service.healthcheck.as_ref().map(|health| HealthConfig {
                test: Some(
                    std::iter::once("CMD".to_owned())
                        .chain(health.command.iter().cloned())
                        .collect(),
                ),
                interval: Some(i64::from(health.interval_secs) * 1_000_000_000),
                retries: Some(i64::from(health.retries)),
                ..Default::default()
            }),
            host_config: Some(host_config),
            networking_config: Some(NetworkingConfig {
                endpoints_config: Some(HashMap::from([(
                    network,
                    EndpointSettings {
                        aliases: Some(vec![service.name.clone()]),
                        ..Default::default()
                    },
                )])),
            }),
            ..Default::default()
        };
        let options = CreateContainerOptionsBuilder::new().name(&name).build();
        self.client.create_container(Some(options), body).await?;
        Ok(())
    }

    /// Conteneur d'installation (ADR-0017) : mêmes durcissements qu'un service (non-root,
    /// `cap_drop ALL`, `no-new-privileges`, limites), volumes du service montés, réseau du
    /// serveur (ses services sont arrêtés). Sa racine reste inscriptible : SteamCMD se met à jour
    /// dans son dossier ; le conteneur est supprimé après usage.
    pub async fn create_installer(&self, installer: &Installer<'_>) -> Result<String, DockerError> {
        let name = container_name(installer.server_id, INSTALLER);
        self.remove_container(&name).await?;
        let host_config = HostConfig {
            mounts: Some(
                installer
                    .mounts
                    .iter()
                    .map(|(target, source)| Mount {
                        target: Some(target.clone()),
                        source: Some(source.to_string_lossy().into_owned()),
                        typ: Some(MountTypeEnum::BIND),
                        read_only: Some(false),
                        ..Default::default()
                    })
                    .collect(),
            ),
            readonly_rootfs: Some(false),
            cap_drop: Some(vec!["ALL".to_owned()]),
            security_opt: Some(vec!["no-new-privileges".to_owned()]),
            memory: i64::try_from(installer.memory_bytes).ok(),
            memory_swap: i64::try_from(installer.memory_bytes).ok(),
            nano_cpus: Some(i64::from(installer.cpu_millis) * 1_000_000),
            pids_limit: Some(i64::from(installer.pids)),
            network_mode: Some(network_name(installer.server_id)),
            log_config: Some(HostConfigLogConfig {
                typ: Some("json-file".to_owned()),
                config: Some(HashMap::from([
                    ("max-size".to_owned(), "10m".to_owned()),
                    ("max-file".to_owned(), "1".to_owned()),
                ])),
            }),
            ..Default::default()
        };
        let body = ContainerCreateBody {
            image: Some(installer.image.to_owned()),
            cmd: Some(installer.command.clone()),
            env: Some(installer.env.clone()),
            user: Some(installer.user.to_owned()),
            labels: Some(labels(installer.server_id, Some(INSTALLER))),
            tty: Some(false),
            host_config: Some(host_config),
            ..Default::default()
        };
        let options = CreateContainerOptionsBuilder::new().name(&name).build();
        self.client.create_container(Some(options), body).await?;
        Ok(name)
    }

    /// Dépose un fichier dans un conteneur créé (archive envoyée à l'API Docker) : il ne passe
    /// jamais par le disque de l'hôte ni par `docker inspect`, et disparaît avec le conteneur.
    pub async fn put_file(
        &self,
        container: &str,
        file: &ContainerFile<'_>,
    ) -> Result<(), DockerError> {
        let mut header = tar::Header::new_gnu();
        header.set_size(u64::try_from(file.content.len()).unwrap_or(u64::MAX));
        header.set_mode(file.mode);
        header.set_uid(file.owner.0);
        header.set_gid(file.owner.1);
        header.set_cksum();
        let mut archive = tar::Builder::new(Vec::new());
        archive.append_data(&mut header, file.name, file.content)?;
        let archive = archive.into_inner()?;
        let options = UploadToContainerOptionsBuilder::new()
            .path(file.dir)
            .build();
        self.client
            .upload_to_container(container, Some(options), bollard::body_full(archive.into()))
            .await?;
        Ok(())
    }

    /// Attend la fin du conteneur ; retourne son code de sortie.
    pub async fn wait(&self, name: &str) -> Result<i64, DockerError> {
        let mut wait = self.client.wait_container(name, None);
        match wait.next().await {
            Some(Ok(response)) => Ok(response.status_code),
            Some(Err(bollard::errors::Error::DockerContainerWaitError { code, .. })) => Ok(code),
            Some(Err(error)) => Err(error.into()),
            None => Ok(0),
        }
    }

    pub async fn remove_container(&self, name: &str) -> Result<(), DockerError> {
        let options = RemoveContainerOptionsBuilder::new().force(true).build();
        match self.client.remove_container(name, Some(options)).await {
            Err(error) if !is_not_found(&error) => Err(error.into()),
            _ => Ok(()),
        }
    }

    pub async fn start(&self, name: &str) -> Result<(), DockerError> {
        self.client.start_container(name, None).await?;
        Ok(())
    }

    /// Arrêt avec signal et délai de grâce, puis SIGKILL.
    pub async fn stop(
        &self,
        name: &str,
        signal: &str,
        timeout_secs: u32,
    ) -> Result<(), DockerError> {
        let options = StopContainerOptionsBuilder::new()
            .signal(signal)
            .t(i32::try_from(timeout_secs).unwrap_or(i32::MAX))
            .build();
        match self.client.stop_container(name, Some(options)).await {
            Err(error) if !is_not_found(&error) && !is_not_modified(&error) => Err(error.into()),
            _ => Ok(()),
        }
    }

    /// Limites d'un conteneur existant, appliquées sans redémarrage (ADR-0019). Sans
    /// conteneur, rien à faire : la création reprendra la spécification gardée.
    pub async fn update_limits(&self, name: &str, limits: &Limits) -> Result<(), DockerError> {
        let body = ContainerUpdateBody {
            memory: i64::try_from(limits.memory_bytes).ok(),
            memory_swap: i64::try_from(limits.memory_bytes).ok(),
            nano_cpus: Some(i64::from(limits.cpu_millis) * 1_000_000),
            pids_limit: Some(i64::from(limits.pids)),
            ..Default::default()
        };
        match self.client.update_container(name, body).await {
            Err(error) if !is_not_found(&error) => Err(error.into()),
            _ => Ok(()),
        }
    }

    pub async fn kill(&self, name: &str) -> Result<(), DockerError> {
        let options = KillContainerOptionsBuilder::new().signal("SIGKILL").build();
        match self.client.kill_container(name, Some(options)).await {
            // 409 : déjà arrêté.
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 409, ..
            }) => Ok(()),
            Err(error) if !is_not_found(&error) => Err(error.into()),
            _ => Ok(()),
        }
    }

    pub async fn status(&self, name: &str) -> Result<Option<ContainerStatus>, DockerError> {
        let inspect = match self.client.inspect_container(name, None).await {
            Ok(inspect) => inspect,
            Err(error) if is_not_found(&error) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let state = inspect.state.unwrap_or_default();
        Ok(Some(ContainerStatus {
            running: state.running.unwrap_or(false),
            exit_code: state.exit_code,
            healthy: state
                .health
                .and_then(|h| h.status)
                .and_then(|status| match status {
                    HealthStatusEnum::HEALTHY => Some(true),
                    HealthStatusEnum::UNHEALTHY => Some(false),
                    _ => None,
                }),
        }))
    }

    /// Écrit une ligne sur l'entrée de la console du conteneur.
    pub async fn send_line(&self, name: &str, line: &str) -> Result<(), DockerError> {
        let options = AttachContainerOptionsBuilder::new()
            .stream(true)
            .stdin(true)
            .build();
        let mut attached = self.client.attach_container(name, Some(options)).await?;
        let io = |error: std::io::Error| {
            DockerError::Api(bollard::errors::Error::IOError { err: error })
        };
        attached
            .input
            .write_all(format!("{line}\n").as_bytes())
            .await
            .map_err(io)?;
        attached.input.flush().await.map_err(io)?;
        Ok(())
    }

    /// Lignes de sortie depuis `since` (horodatage Unix, en secondes), stdout et stderr.
    pub fn follow_lines(
        &self,
        name: &str,
        since: i64,
    ) -> impl Stream<Item = String> + Send + 'static {
        let options = LogsOptionsBuilder::new()
            .follow(true)
            .stdout(true)
            .stderr(true)
            .since(i32::try_from(since).unwrap_or(0))
            .build();
        let chunks = self.client.logs(name, Some(options));
        futures_util::stream::unfold(
            (
                Box::pin(chunks),
                String::new(),
                std::collections::VecDeque::new(),
            ),
            |(mut chunks, mut buffer, mut ready)| async move {
                loop {
                    if let Some(line) = ready.pop_front() {
                        return Some((line, (chunks, buffer, ready)));
                    }
                    let chunk = chunks.next().await?.ok()?;
                    buffer.push_str(&String::from_utf8_lossy(&chunk.into_bytes()));
                    for line in drain_lines(&mut buffer) {
                        ready.push_back(line.trim_end().to_owned());
                    }
                }
            },
        )
    }

    /// Événements des conteneurs Kuro (démarrage, arrêt, santé).
    pub fn events(&self) -> impl Stream<Item = ContainerEvent> + Send + 'static {
        let filters = HashMap::from([
            ("type", vec!["container"]),
            ("label", vec!["kuro.managed=true"]),
            ("event", vec!["start", "die", "health_status"]),
        ]);
        let options = EventsOptionsBuilder::new().filters(&filters).build();
        self.client
            .events(Some(options))
            .filter_map(|event| async move {
                let event = event.ok()?;
                let attributes = event.actor?.attributes?;
                Some(ContainerEvent {
                    server_id: attributes.get(LABEL_SERVER)?.parse().ok()?,
                    service: attributes.get(LABEL_SERVICE)?.clone(),
                    action: event.action?,
                    exit_code: attributes
                        .get("exitCode")
                        .and_then(|code| code.parse().ok()),
                })
            })
    }

    /// Serveurs dont des conteneurs Kuro existent sur l'hôte.
    /// Crée le conteneur du proxy (sans le démarrer), sur le pont par défaut.
    pub async fn create_proxy(&self, spec: &ProxyContainer<'_>) -> Result<(), DockerError> {
        let mut exposed = Vec::new();
        let mut bindings: HashMap<String, Option<Vec<PortBinding>>> = HashMap::new();
        for (container_port, host_port) in &spec.ports {
            let key = format!("{container_port}/tcp");
            exposed.push(key.clone());
            bindings.insert(
                key,
                Some(vec![PortBinding {
                    host_ip: Some("0.0.0.0".to_owned()),
                    host_port: Some(host_port.to_string()),
                }]),
            );
        }
        let host_config = HostConfig {
            mounts: Some(
                spec.mounts
                    .iter()
                    .map(|(target, source, read_only)| Mount {
                        target: Some(target.clone()),
                        source: Some(source.to_string_lossy().into_owned()),
                        typ: Some(MountTypeEnum::BIND),
                        read_only: Some(*read_only),
                        ..Default::default()
                    })
                    .collect(),
            ),
            tmpfs: Some(HashMap::from([(
                "/tmp".to_owned(),
                "rw,nosuid,nodev,noexec".to_owned(),
            )])),
            readonly_rootfs: Some(true),
            // Le binaire de l'image porte la capacité de fichier `cap_net_bind_service` : sous
            // `no-new-privileges`, le noyau refuse de l'exécuter (`operation not permitted`).
            // Toutes les capacités sont retirées sauf celle-là, et l'utilisateur n'est pas root.
            cap_drop: Some(vec!["ALL".to_owned()]),
            cap_add: Some(vec!["NET_BIND_SERVICE".to_owned()]),
            memory: i64::try_from(spec.memory_bytes).ok(),
            memory_swap: i64::try_from(spec.memory_bytes).ok(),
            pids_limit: Some(i64::from(spec.pids)),
            port_bindings: Some(bindings),
            network_mode: Some("bridge".to_owned()),
            restart_policy: Some(RestartPolicy {
                name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
                maximum_retry_count: None,
            }),
            log_config: Some(HostConfigLogConfig {
                typ: Some("json-file".to_owned()),
                config: Some(HashMap::from([
                    ("max-size".to_owned(), "10m".to_owned()),
                    ("max-file".to_owned(), "3".to_owned()),
                ])),
            }),
            ..Default::default()
        };
        let mut labels = spec.labels.clone();
        labels.insert(LABEL_MANAGED.to_owned(), "proxy".to_owned());
        let body = ContainerCreateBody {
            image: Some(spec.image.to_owned()),
            cmd: Some(spec.command.clone()),
            env: Some(spec.env.clone()),
            user: Some(spec.user.to_owned()),
            labels: Some(labels),
            exposed_ports: Some(exposed),
            host_config: Some(host_config),
            ..Default::default()
        };
        let options = CreateContainerOptionsBuilder::new().name(spec.name).build();
        self.client.create_container(Some(options), body).await?;
        Ok(())
    }

    /// Étiquettes d'un conteneur ; `None` s'il n'existe pas.
    pub async fn labels_of(
        &self,
        name: &str,
    ) -> Result<Option<HashMap<String, String>>, DockerError> {
        match self.client.inspect_container(name, None).await {
            Ok(inspect) => Ok(Some(
                inspect.config.and_then(|c| c.labels).unwrap_or_default(),
            )),
            Err(error) if is_not_found(&error) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Réseaux auxquels un conteneur est relié (vide s'il n'existe pas).
    pub async fn networks_of(&self, name: &str) -> Result<Vec<String>, DockerError> {
        match self.client.inspect_container(name, None).await {
            Ok(inspect) => Ok(inspect
                .network_settings
                .and_then(|settings| settings.networks)
                .map(|networks| networks.into_keys().collect())
                .unwrap_or_default()),
            Err(error) if is_not_found(&error) => Ok(Vec::new()),
            Err(error) => Err(error.into()),
        }
    }

    pub async fn connect_network(&self, container: &str, network: &str) -> Result<(), DockerError> {
        self.client
            .connect_network(
                network,
                NetworkConnectRequest {
                    container: container.to_owned(),
                    endpoint_config: None,
                },
            )
            .await?;
        Ok(())
    }

    pub async fn disconnect_network(
        &self,
        container: &str,
        network: &str,
    ) -> Result<(), DockerError> {
        match self
            .client
            .disconnect_network(
                network,
                NetworkDisconnectRequest {
                    container: container.to_owned(),
                    force: Some(true),
                },
            )
            .await
        {
            Err(error) if !is_not_found(&error) => Err(error.into()),
            _ => Ok(()),
        }
    }

    /// Exécute une commande dans un conteneur en marche : code de sortie et sortie (stdout et
    /// stderr), tronquée à 4 Kio.
    pub async fn exec(
        &self,
        container: &str,
        cmd: Vec<String>,
    ) -> Result<(i64, String), DockerError> {
        let exec = self
            .client
            .create_exec(
                container,
                ExecConfig {
                    cmd: Some(cmd),
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    ..Default::default()
                },
            )
            .await?;
        let mut output = String::new();
        if let bollard::exec::StartExecResults::Attached {
            output: mut stream, ..
        } = self.client.start_exec(&exec.id, None).await?
        {
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                if output.len() < 4096 {
                    output.push_str(&String::from_utf8_lossy(&chunk.into_bytes()));
                }
            }
        }
        let inspect = self.client.inspect_exec(&exec.id).await?;
        Ok((inspect.exit_code.unwrap_or(-1), output))
    }

    pub async fn kuro_servers(&self) -> Result<Vec<Uuid>, DockerError> {
        let filters = HashMap::from([("label", vec!["kuro.managed=true"])]);
        let options = ListContainersOptionsBuilder::new()
            .all(true)
            .filters(&filters)
            .build();
        let mut servers: Vec<Uuid> = self
            .client
            .list_containers(Some(options))
            .await?
            .into_iter()
            .filter_map(|c| c.labels?.get(LABEL_SERVER)?.parse().ok())
            .collect();
        servers.sort();
        servers.dedup();
        Ok(servers)
    }

    /// Attribue des chemins à `uid:gid` via un conteneur utilitaire éphémère (sans réseau,
    /// seule capacité `CHOWN`) : même mécanisme sous Linux et sous Docker Desktop.
    /// `targets` : (dossier hôte monté, chemins relatifs à attribuer, `.` pour le dossier).
    pub async fn chown(
        &self,
        targets: &[(PathBuf, Vec<String>)],
        uid: u32,
        gid: u32,
    ) -> Result<(), DockerError> {
        if targets.is_empty() {
            return Ok(());
        }
        self.ensure_image(&self.helper_image).await?;
        let mut command = vec!["chown".to_owned(), format!("{uid}:{gid}")];
        let mut mounts = Vec::new();
        for (index, (dir, paths)) in targets.iter().enumerate() {
            let target = format!("/kuro/{index}");
            for path in paths {
                command.push(if path == "." {
                    target.clone()
                } else {
                    format!("{target}/{path}")
                });
            }
            mounts.push(Mount {
                target: Some(target),
                source: Some(dir.to_string_lossy().into_owned()),
                typ: Some(MountTypeEnum::BIND),
                read_only: Some(false),
                ..Default::default()
            });
        }
        let name = format!("kuro-helper-{}", Uuid::now_v7());
        let body = ContainerCreateBody {
            image: Some(self.helper_image.clone()),
            cmd: Some(command),
            user: Some("0:0".to_owned()),
            labels: Some(HashMap::from([(
                LABEL_MANAGED.to_owned(),
                "helper".to_owned(),
            )])),
            host_config: Some(HostConfig {
                mounts: Some(mounts),
                network_mode: Some("none".to_owned()),
                readonly_rootfs: Some(true),
                cap_drop: Some(vec!["ALL".to_owned()]),
                cap_add: Some(vec!["CHOWN".to_owned()]),
                security_opt: Some(vec!["no-new-privileges".to_owned()]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let options = CreateContainerOptionsBuilder::new().name(&name).build();
        self.client.create_container(Some(options), body).await?;
        let outcome = async {
            self.client.start_container(&name, None).await?;
            let mut wait = self.client.wait_container(&name, None);
            let code = match wait.next().await {
                Some(Ok(response)) => response.status_code,
                Some(Err(bollard::errors::Error::DockerContainerWaitError { code, .. })) => code,
                Some(Err(error)) => return Err(error.into()),
                None => 0,
            };
            if code == 0 {
                Ok(())
            } else {
                Err(DockerError::Helper(code))
            }
        }
        .await;
        self.remove_container(&name).await?;
        outcome
    }
    /// Console horodatée : les `tail` dernières lignes puis le direct, ou le direct depuis
    /// `since` (secondes Unix). Le flux se termine quand le conteneur s'arrête.
    pub fn console_lines(
        &self,
        name: &str,
        from: ConsoleFrom,
    ) -> impl Stream<Item = TimedLine> + Send + 'static {
        let builder = LogsOptionsBuilder::new()
            .follow(true)
            .stdout(true)
            .stderr(true)
            .timestamps(true);
        let options = match from {
            ConsoleFrom::Tail(lines) => builder.tail(&lines.to_string()),
            ConsoleFrom::Since(secs) => builder.since(i32::try_from(secs).unwrap_or(0)),
        }
        .build();
        let chunks = self.client.logs(name, Some(options));
        futures_util::stream::unfold(
            (
                Box::pin(chunks),
                String::new(),
                std::collections::VecDeque::new(),
            ),
            |(mut chunks, mut buffer, mut ready)| async move {
                loop {
                    if let Some(line) = ready.pop_front() {
                        return Some((line, (chunks, buffer, ready)));
                    }
                    let chunk = chunks.next().await?.ok()?;
                    buffer.push_str(&String::from_utf8_lossy(&chunk.into_bytes()));
                    for line in drain_lines(&mut buffer) {
                        ready.push_back(TimedLine::parse(line.trim_end_matches(['\r', '\n'])));
                    }
                }
            },
        )
    }

    /// Mesure ponctuelle d'un conteneur (Docker l'étale sur environ une seconde pour calculer le
    /// CPU). `None` si le conteneur n'existe pas ou ne tourne pas.
    pub async fn stats(&self, name: &str) -> Option<ContainerStatsResponse> {
        let options = StatsOptionsBuilder::new()
            .stream(false)
            .one_shot(false)
            .build();
        let mut stats = Box::pin(self.client.stats(name, Some(options)));
        stats.next().await?.ok()
    }
}

fn is_not_modified(error: &bollard::errors::Error) -> bool {
    matches!(
        error,
        bollard::errors::Error::DockerResponseServerError {
            status_code: 304,
            ..
        }
    )
}

fn labels(server_id: Uuid, service: Option<&str>) -> HashMap<String, String> {
    let mut labels = HashMap::from([
        (LABEL_MANAGED.to_owned(), "true".to_owned()),
        (LABEL_SERVER.to_owned(), server_id.to_string()),
    ]);
    if let Some(service) = service {
        labels.insert(LABEL_SERVICE.to_owned(), service.to_owned());
    }
    labels
}

/// Lignes complètes du tampon, dans l'ordre. Un flux sans retour à la ligne (barre de progression
/// en `\r`, sortie binaire) ne fait pas grossir le tampon sans fin : au-delà de
/// [`MAX_CONSOLE_LINE_BYTES`], ce qui est là forme une ligne.
fn drain_lines(buffer: &mut String) -> Vec<String> {
    let mut lines = Vec::new();
    while let Some(index) = buffer.find('\n') {
        lines.push(buffer.drain(..=index).collect());
    }
    if buffer.len() > MAX_CONSOLE_LINE_BYTES {
        lines.push(std::mem::take(buffer));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_split_on_newlines_and_a_runaway_buffer_is_flushed() {
        let mut buffer = String::from("one\ntwo\npartial");
        assert_eq!(drain_lines(&mut buffer), ["one\n", "two\n"]);
        assert_eq!(buffer, "partial");

        // Barre de progression : des kilo-octets de `\r` sans jamais de `\n`.
        buffer.push_str(&"\r50%".repeat(MAX_CONSOLE_LINE_BYTES));
        let flushed = drain_lines(&mut buffer);
        assert_eq!(flushed.len(), 1);
        assert!(flushed[0].starts_with("partial\r50%"));
        assert!(buffer.is_empty());
    }
}
