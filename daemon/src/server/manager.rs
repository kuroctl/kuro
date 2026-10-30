//! Exécution des commandes de l'API sur les serveurs du nœud (ADR-0013).
//!
//! Une commande est validée puis **acceptée tout de suite** (`command.result`) ; elle s'exécute
//! ensuite en tâche de fond et se termine par des `server.state`. Une seule opération à la fois
//! par serveur (`busy`) ; une sauvegarde en cours n'exclut que les opérations sur les données
//! (pas les actions d'alimentation). L'attente de l'état « prêt » ne bloque pas le serveur : on peut
//! l'arrêter pendant son démarrage.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures_util::StreamExt;
use kuro_protocol::DaemonMessage;
use kuro_protocol::api::server::{PowerAction, ReadySpec, ServerDeploy, StopSpec};
use kuro_protocol::daemon::server::{ServerState, ServerStateReport};
use kuro_protocol::daemon::{BackupDone, ContentDone};
use regex::Regex;
use tokio::sync::{Semaphore, mpsc};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::spec;
use crate::docker::{ContainerEvent, Docker, DockerError, container_name};
use crate::fs::{FsError, ServerDir};
use crate::install::download::DownloadPolicy;

mod backup;
mod content;
mod disk;
mod files;
mod install;
mod states;
mod upload;
mod watch;

pub use states::StateReceiver;

/// Utilisateur `(uid, gid)` → (dossier de volume, chemins relatifs) à lui attribuer.
type Ownership = BTreeMap<(u32, u32), Vec<(PathBuf, Vec<String>)>>;

/// Délai de grâce des services secondaires (base de données…), arrêtés après le principal.
const SECONDARY_STOP_SECS: u32 = 20;
const POLL: Duration = Duration::from_secs(1);
/// Messages de console et de stats en attente d'envoi ; au-delà, ils sont perdus (et comptés).
const STREAM_QUEUE: usize = 64;
/// Opérations lourdes menées de front sur l'hôte ; les suivantes attendent leur tour, commande
/// déjà acceptée. Deux cents serveurs qui s'installent ensemble ne saturent ni le disque ni le
/// réseau.
const MAX_PARALLEL_INSTALLS: usize = 4;
const MAX_PARALLEL_BACKUPS: usize = 2;
const MAX_PARALLEL_CONTENT: usize = 4;
/// Mesures d'espace disque de front (chacune parcourt tous les fichiers d'un serveur).
const MAX_PARALLEL_MEASURES: usize = 2;

/// Messages produits par le gestionnaire, à transmettre à l'API.
#[derive(Debug)]
pub struct Receivers {
    /// Dernier état de chaque serveur, jamais perdu (coalescé par serveur).
    pub events: StateReceiver,
    pub streams: mpsc::Receiver<DaemonMessage>,
    /// Fins de sauvegarde (ADR-0018), à ne pas perdre comme les états.
    pub backups: mpsc::Receiver<BackupDone>,
    /// Fins d'installation de contenu (ADR-0020).
    pub content: mpsc::Receiver<ContentDone>,
}

/// Refus immédiat d'une commande.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CommandError {
    #[error("invalid spec: {0}")]
    InvalidSpec(String),
    #[error("unknown server")]
    UnknownServer,
    #[error("an operation is already running on this server")]
    Busy,
    #[error("server is not running")]
    NotRunning,
    #[error("path refused")]
    InvalidPath,
    #[error("no such file or directory")]
    NotFound,
    #[error("already exists")]
    AlreadyExists,
    #[error("file changed since it was read")]
    Conflict,
    #[error("not a text file")]
    NotText,
    #[error("file too large")]
    TooLarge,
    #[error("disk quota exceeded")]
    DiskQuotaExceeded,
    #[error("server is suspended")]
    Suspended,
    #[error("{0}")]
    Internal(String),
}

#[derive(Debug, thiserror::Error)]
pub(super) enum OpError {
    #[error(transparent)]
    Docker(#[from] DockerError),
    #[error(transparent)]
    Fs(#[from] FsError),
    #[error("{0}")]
    Spec(String),
}

#[derive(Debug, Clone)]
pub struct Manager {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    docker: Docker,
    data_root: PathBuf,
    /// Proxy des sites web du nœud (ADR-0038).
    proxy: crate::web::Proxy,
    busy: Mutex<HashSet<Uuid>>,
    /// Dernier état annoncé par serveur : renvoyé tel quel à une reconnexion pendant une
    /// opération (`installing`…), le conteneur principal ne reflétant pas encore son issue.
    announced: Mutex<HashMap<Uuid, (ServerState, Option<String>)>>,
    /// Serveur → sauvegarde en cours d'archivage.
    archiving: Mutex<HashMap<Uuid, Uuid>>,
    /// Serveurs arrêtés volontairement : leur arrêt n'est pas un incident.
    stopping: Mutex<HashSet<Uuid>>,
    events: states::StateSender,
    installs: Arc<Semaphore>,
    backup_slots: Arc<Semaphore>,
    content_slots: Arc<Semaphore>,
    measure_slots: Arc<Semaphore>,
    /// Serveurs dont une mesure d'espace disque est déjà programmée après une écriture.
    measuring: Mutex<HashSet<Uuid>>,
    /// Spécifications déployées, telles qu'écrites sur le disque : lues une fois, jamais sur les
    /// workers à chaque commande ou événement Docker.
    specs: Mutex<HashMap<Uuid, ServerDeploy>>,
    /// Serveurs regardés : annuler le jeton arrête leur console et leurs stats.
    watches: Mutex<HashMap<Uuid, CancellationToken>>,
    /// Console et stats des serveurs regardés (perdables, contrairement aux états).
    streams: mpsc::Sender<DaemonMessage>,
    /// Règles des téléchargements d'installation (ADR-0017).
    downloads: DownloadPolicy,
    backups: mpsc::Sender<BackupDone>,
    /// Dernier espace disque mesuré par serveur (ADR-0019).
    disk: Mutex<HashMap<Uuid, u64>>,
    content: mpsc::Sender<ContentDone>,
    /// Envois de fichiers en cours (ADR-0020).
    uploads: Mutex<HashMap<Uuid, upload::Upload>>,
}

impl Manager {
    /// Les récepteurs portent les messages à transmettre à l'API : `server.state` d'une part,
    /// console et stats des serveurs regardés d'autre part.
    pub fn new(docker: Docker, data_root: PathBuf) -> (Self, Receivers) {
        Self::with_downloads(docker, data_root, DownloadPolicy::default())
    }

    /// Comme [`Manager::new`], avec d'autres règles de téléchargement (tests : serveur local).
    pub fn with_downloads(
        docker: Docker,
        data_root: PathBuf,
        downloads: DownloadPolicy,
    ) -> (Self, Receivers) {
        Self::with_options(
            docker,
            data_root,
            downloads,
            crate::web::WebConfig::default(),
        )
    }

    /// Règles de téléchargement et réglages du proxy des sites web (ADR-0038).
    pub fn with_options(
        docker: Docker,
        data_root: PathBuf,
        downloads: DownloadPolicy,
        web: crate::web::WebConfig,
    ) -> (Self, Receivers) {
        let proxy = crate::web::Proxy::new(docker.clone(), &data_root, web);
        let (events, receiver) = states::channel();
        let (streams, streams_receiver) = mpsc::channel(STREAM_QUEUE);
        let (backups, backups_receiver) = mpsc::channel(64);
        let (content, content_receiver) = mpsc::channel(64);
        let manager = Self {
            inner: Arc::new(Inner {
                docker,
                data_root,
                proxy,
                busy: Mutex::new(HashSet::new()),
                announced: Mutex::new(HashMap::new()),
                archiving: Mutex::new(HashMap::new()),
                stopping: Mutex::new(HashSet::new()),
                events,
                installs: Arc::new(Semaphore::new(MAX_PARALLEL_INSTALLS)),
                backup_slots: Arc::new(Semaphore::new(MAX_PARALLEL_BACKUPS)),
                content_slots: Arc::new(Semaphore::new(MAX_PARALLEL_CONTENT)),
                measure_slots: Arc::new(Semaphore::new(MAX_PARALLEL_MEASURES)),
                measuring: Mutex::new(HashSet::new()),
                specs: Mutex::new(HashMap::new()),
                watches: Mutex::new(HashMap::new()),
                streams,
                downloads,
                backups,
                disk: Mutex::new(HashMap::new()),
                content,
                uploads: Mutex::new(HashMap::new()),
            }),
        };
        (
            manager,
            Receivers {
                events: receiver,
                streams: streams_receiver,
                backups: backups_receiver,
                content: content_receiver,
            },
        )
    }

    fn dir(&self, server_id: Uuid) -> ServerDir {
        ServerDir::new(&self.inner.data_root, server_id)
    }

    pub fn deploy(&self, deploy: ServerDeploy) -> Result<(), CommandError> {
        spec::check(&deploy).map_err(CommandError::InvalidSpec)?;
        let guard = self.acquire(deploy.server_id)?;
        let this = self.clone();
        tokio::spawn(async move {
            let id = deploy.server_id;
            // Différé : un serveur en marche le reste ; `run_deploy` annonce ce qu'il fait.
            if !deploy.defer {
                this.report(id, ServerState::Deploying, None);
            }
            if let Err(error) = this.run_deploy(&deploy).await {
                tracing::warn!(server_id = %id, %error, "deploy failed");
                this.report(id, ServerState::Failed, Some(error.to_string()));
            }
            drop(guard);
        });
        Ok(())
    }

    pub fn delete(&self, server_id: Uuid) -> Result<(), CommandError> {
        let guard = self.acquire(server_id)?;
        self.unwatch(server_id);
        let this = self.clone();
        tokio::spawn(async move {
            match this.run_delete(server_id).await {
                Ok(()) => this.report(server_id, ServerState::Deleted, None),
                Err(error) => {
                    tracing::warn!(%server_id, %error, "delete failed");
                    this.report(server_id, ServerState::Failed, Some(error.to_string()));
                }
            }
            drop(guard);
        });
        Ok(())
    }

    /// Action d'un prefab (ADR-0039) : la commande, déjà rendue par l'API, s'exécute dans le
    /// conteneur du service en marche avec l'utilisateur du service, dans la durée demandée.
    /// Refusée pendant une autre opération, sur un serveur arrêté, ou vers un service que la
    /// spécification déployée ne déclare pas.
    pub async fn action(
        &self,
        action: kuro_protocol::api::server::ServerAction,
    ) -> Result<kuro_protocol::daemon::ActionResult, CommandError> {
        let server_id = action.server_id;
        let deploy = self.spec(server_id)?;
        let service = deploy
            .services
            .iter()
            .find(|s| s.name == action.service)
            .ok_or_else(|| {
                CommandError::InvalidSpec(format!("unknown service `{}`", action.service))
            })?;
        if action.command.is_empty()
            || action
                .command
                .iter()
                .any(|arg| arg.contains('\0') || arg.len() > 4096)
        {
            return Err(CommandError::InvalidSpec(
                "invalid action command".to_owned(),
            ));
        }
        let timeout = Duration::from_secs(u64::from(
            action
                .timeout_secs
                .clamp(1, kuro_protocol::limits::MAX_ACTION_TIMEOUT_SECS),
        ));
        let guard = self.acquire(server_id)?;
        let docker = &self.inner.docker;
        let container = container_name(server_id, &service.name);
        let running = docker
            .status(&container)
            .await
            .map_err(|e| CommandError::Internal(e.to_string()))?
            .is_some_and(|s| s.running);
        if !running {
            return Err(CommandError::NotRunning);
        }
        tracing::info!(%server_id, action = %action.action, service = %service.name, "running prefab action");
        let outcome = tokio::time::timeout(
            timeout,
            docker.exec(&container, action.command, Some(&service.user)),
        )
        .await;
        drop(guard);
        match outcome {
            Ok(Ok(outcome)) => Ok(kuro_protocol::daemon::ActionResult {
                server_id,
                exit_code: outcome.exit_code,
                output: outcome.output,
                truncated: outcome.truncated,
            }),
            Ok(Err(error)) => Err(CommandError::Internal(error.to_string())),
            Err(_) => Err(CommandError::Internal(format!(
                "action timed out after {} s",
                timeout.as_secs()
            ))),
        }
    }

    pub async fn power(&self, server_id: Uuid, action: PowerAction) -> Result<(), CommandError> {
        let deploy = self.spec(server_id)?;
        if matches!(action, PowerAction::Start | PowerAction::Restart) {
            disk::ensure_active(&deploy)?;
            self.check_quota(&deploy, 0).await?;
        }
        let guard = self.acquire_power(server_id)?;
        let this = self.clone();
        tokio::spawn(async move {
            let outcome = match action {
                PowerAction::Start => this.start_applying(&deploy).await,
                PowerAction::Stop => this.stop_all(&deploy).await,
                PowerAction::Restart => match this.stop_all(&deploy).await {
                    Ok(()) => this.start_applying(&deploy).await,
                    Err(error) => Err(error),
                },
                PowerAction::Kill => this.kill_all(&deploy).await,
            };
            if let Err(error) = outcome {
                tracing::warn!(%server_id, ?action, %error, "power action failed");
                this.report(server_id, ServerState::Failed, Some(error.to_string()));
            }
            drop(guard);
        });
        Ok(())
    }

    /// Après une reconnexion : état réel de chaque serveur assigné, conteneurs inconnus signalés.
    pub fn sync(&self, server_ids: Vec<Uuid>) {
        let this = self.clone();
        tokio::spawn(async move {
            for &server_id in &server_ids {
                // Opération en cours : son état annoncé fait foi, elle annoncera son issue.
                let pending = this.inner.busy().contains(&server_id);
                if let Some((state, reason)) = pending
                    .then(|| this.inner.announced().get(&server_id).cloned())
                    .flatten()
                {
                    this.report(server_id, state, reason);
                    continue;
                }
                let state = this.observed_state(server_id).await;
                let (state, reason) = match state {
                    Ok(Some(state)) => (state, None),
                    Ok(None) => (
                        ServerState::Failed,
                        Some("not deployed on this node".to_owned()),
                    ),
                    Err(error) => (ServerState::Failed, Some(error.to_string())),
                };
                this.report(server_id, state, reason);
            }
            // Espace disque à jour pour l'API dès la reconnexion.
            for &server_id in &server_ids {
                this.measure(server_id).await;
            }
            if let Ok(found) = this.inner.docker.kuro_servers().await {
                for unknown in found.iter().filter(|id| !server_ids.contains(id)) {
                    // Jamais supprimé automatiquement : un admin décide.
                    tracing::warn!(server_id = %unknown, "kuro containers of a server unknown to the api");
                }
            }
        });
    }

    /// Suit les événements Docker : un arrêt inattendu du service principal est signalé.
    pub async fn watch_events(&self) {
        loop {
            let mut events = Box::pin(self.inner.docker.events());
            while let Some(event) = events.next().await {
                self.on_event(event);
            }
            // Flux coupé (Docker redémarré) : on se réabonne.
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    fn on_event(&self, event: ContainerEvent) {
        if event.action != "die"
            || self.is_busy(event.server_id)
            || self.is_stopping(event.server_id)
        {
            return;
        }
        let Ok(Some(deploy)) = self.spec_opt(event.server_id) else {
            return;
        };
        if deploy.lifecycle.service == event.service {
            let reason = event
                .exit_code
                .filter(|code| *code != 0)
                .map(|code| format!("exited with code {code}"));
            self.report(event.server_id, ServerState::Offline, reason);
        }
    }

    async fn observed_state(&self, server_id: Uuid) -> Result<Option<ServerState>, OpError> {
        let Some(deploy) = self.spec_opt(server_id)? else {
            return Ok(None);
        };
        let main = container_name(server_id, &deploy.lifecycle.service);
        Ok(Some(match self.inner.docker.status(&main).await? {
            Some(status) if status.running => ServerState::Running,
            _ => ServerState::Offline,
        }))
    }

    async fn run_deploy(&self, deploy: &ServerDeploy) -> Result<(), OpError> {
        let docker = &self.inner.docker;
        let id = deploy.server_id;
        let mut images: Vec<&str> = deploy.services.iter().map(|s| s.image.as_str()).collect();
        images.sort_unstable();
        images.dedup();
        for image in images {
            // Un tag est retiré à chaque déploiement : les correctifs de l'image suivent.
            docker.refresh_image(image).await?;
        }

        let dir = self.dir(id);
        // La spécification gardée sert aux actions suivantes ; l'installation n'en fait pas partie.
        let previous = self.spec_opt(id)?;
        let kept = ServerDeploy {
            install: None,
            ..deploy.clone()
        };
        let volumes_of = deploy.clone();
        let written = tokio::task::spawn_blocking({
            let dir = dir.clone();
            let kept = kept.clone();
            move || -> Result<(), FsError> {
                dir.ensure_volumes(&volumes_of)?;
                dir.save_spec(&kept)
            }
        })
        .await
        .map_err(|e| OpError::Spec(e.to_string()))?;
        written?;
        self.inner.specs().insert(id, kept.clone());
        self.give_volumes(deploy).await?;
        docker.ensure_network(id).await?;
        // Site web (ADR-0038) : le proxy du nœud reçoit les routes du serveur. Un échec ne retient
        // pas le déploiement : il est porté par l'état web du serveur.
        self.inner.proxy.set_routes(id, deploy.web.clone()).await;

        let main = container_name(id, &deploy.lifecycle.service);
        let was_running = docker.status(&main).await?.is_some_and(|s| s.running);
        // Même spécification qu'avant sur un serveur en marche (commande rejouée après une
        // reconnexion) : rien à recréer, le serveur n'est pas redémarré.
        let unchanged = previous.is_some_and(|previous| {
            ServerDeploy {
                defer: false,
                ..previous
            } == ServerDeploy {
                defer: false,
                ..kept.clone()
            }
        });
        if was_running && unchanged && deploy.install.is_none() && !dir.is_pending() {
            self.report(id, ServerState::Running, None);
            return Ok(());
        }
        // Différé sur un serveur en marche : rien n'est arrêté, les conteneurs seront recréés
        // au prochain démarrage (changement de contenu, ADR-0030).
        if deploy.defer && was_running && deploy.install.is_none() {
            dir.set_pending(true)?;
            return Ok(());
        }
        if deploy.defer {
            self.report(id, ServerState::Deploying, None);
        }
        if was_running {
            self.stop_all(deploy).await?;
        }
        self.create_services(deploy).await?;
        if let Some(install) = &deploy.install {
            // Première installation, d'un seul tenant avec la création (ADR-0017).
            let _slot = self.inner.installs.clone().acquire_owned().await;
            let outcome = self.run_steps(deploy, install).await;
            self.finish_install(id, outcome);
            return Ok(());
        }
        if was_running {
            self.start_all(deploy).await
        } else {
            self.report(id, ServerState::Offline, None);
            Ok(())
        }
    }

    /// Conteneurs du serveur (re)créés d'après sa spécification ; une spécification en attente
    /// est alors appliquée.
    async fn create_services(&self, deploy: &ServerDeploy) -> Result<(), OpError> {
        let id = deploy.server_id;
        let dir = self.dir(id);
        for service in &deploy.services {
            let mounts: Vec<(String, PathBuf)> = service
                .volumes
                .iter()
                .map(|v| (v.path.clone(), dir.volume_dir(&service.name, &v.name)))
                .collect();
            self.inner
                .docker
                .create_service(id, service, &mounts)
                .await?;
        }
        dir.set_pending(false)?;
        Ok(())
    }

    /// Démarrage d'un serveur arrêté : une spécification en attente recrée d'abord ses
    /// conteneurs.
    async fn start_applying(&self, deploy: &ServerDeploy) -> Result<(), OpError> {
        if self.dir(deploy.server_id).is_pending() {
            self.create_services(deploy).await?;
        }
        self.start_all(deploy).await
    }

    /// Chaque volume, contenu compris, à l'utilisateur de son service.
    pub(super) async fn give_volumes(&self, deploy: &ServerDeploy) -> Result<(), OpError> {
        let dir = self.dir(deploy.server_id);
        let mut ownership = Ownership::new();
        for service in &deploy.services {
            let owner = owner_of(deploy, &service.name)?;
            for volume in &service.volumes {
                ownership.entry(owner).or_default().push((
                    dir.volume_dir(&service.name, &volume.name),
                    vec![".".to_owned()],
                ));
            }
        }
        for ((uid, gid), targets) in ownership {
            self.inner.docker.chown(&targets, uid, gid).await?;
        }
        Ok(())
    }

    /// Écrit les fichiers, démarre les services dans l'ordre des dépendances, puis surveille
    /// l'état « prêt » sans bloquer le serveur.
    async fn start_all(&self, deploy: &ServerDeploy) -> Result<(), OpError> {
        let docker = &self.inner.docker;
        let id = deploy.server_id;
        self.report(id, ServerState::Starting, None);
        self.write_files(deploy).await?;
        let order = spec::start_order(deploy)
            .ok_or_else(|| OpError::Spec("dependency cycle".to_owned()))?;
        let since = unix_now() - 1;
        for service in order {
            docker.start(&container_name(id, service)).await?;
        }
        self.inner.stopping_remove(id);
        let this = self.clone();
        let deploy = deploy.clone();
        tokio::spawn(async move { this.watch_ready(&deploy, since).await });
        Ok(())
    }

    async fn watch_ready(&self, deploy: &ServerDeploy, since: i64) {
        let id = deploy.server_id;
        let main = container_name(id, &deploy.lifecycle.service);
        let docker = &self.inner.docker;
        let ready = match &deploy.lifecycle.ready {
            None => true,
            Some(ReadySpec::Console(pattern)) => {
                let Ok(pattern) = Regex::new(pattern) else {
                    return;
                };
                let mut lines = Box::pin(docker.follow_lines(&main, since));
                let mut found = false;
                while let Some(line) = lines.next().await {
                    if pattern.is_match(&line) {
                        found = true;
                        break;
                    }
                }
                found
            }
            Some(ReadySpec::Healthcheck) => loop {
                match docker.status(&main).await {
                    Ok(Some(status)) if !status.running => break false,
                    Ok(Some(status)) if status.healthy == Some(true) => break true,
                    Ok(Some(_)) => tokio::time::sleep(POLL).await,
                    _ => break false,
                }
            },
        };
        // Arrêté pendant le démarrage : l'état a déjà été signalé par l'arrêt.
        let still_running = docker
            .status(&main)
            .await
            .ok()
            .flatten()
            .is_some_and(|s| s.running);
        if ready && still_running && !self.is_stopping(id) {
            self.report(id, ServerState::Running, None);
        }
    }

    async fn stop_all(&self, deploy: &ServerDeploy) -> Result<(), OpError> {
        let docker = &self.inner.docker;
        let id = deploy.server_id;
        self.inner.stopping_insert(id);
        self.report(id, ServerState::Stopping, None);
        let lifecycle = &deploy.lifecycle;
        let main = container_name(id, &lifecycle.service);
        match &lifecycle.stop {
            StopSpec::Console(command) => {
                if docker.status(&main).await?.is_some_and(|s| s.running) {
                    if let Err(error) = docker.send_line(&main, command).await {
                        tracing::warn!(server_id = %id, %error, "console stop command failed");
                    }
                    let deadline = tokio::time::Instant::now()
                        + Duration::from_secs(lifecycle.stop_timeout_secs.into());
                    while tokio::time::Instant::now() < deadline
                        && docker.status(&main).await?.is_some_and(|s| s.running)
                    {
                        tokio::time::sleep(POLL).await;
                    }
                }
                docker.kill(&main).await?;
            }
            StopSpec::Signal(signal) => {
                docker
                    .stop(&main, Some(signal.as_str()), lifecycle.stop_timeout_secs)
                    .await?;
            }
            // Le `STOPSIGNAL` de l'image (ADR-0039) : Docker le connaît, pas le daemon.
            StopSpec::Image => {
                docker
                    .stop(&main, None, lifecycle.stop_timeout_secs)
                    .await?;
            }
        }
        let order = spec::start_order(deploy).unwrap_or_default();
        for service in order.into_iter().rev().filter(|s| *s != lifecycle.service) {
            docker
                .stop(
                    &container_name(id, service),
                    Some("SIGTERM"),
                    SECONDARY_STOP_SECS,
                )
                .await?;
        }
        self.report(id, ServerState::Offline, None);
        Ok(())
    }

    async fn kill_all(&self, deploy: &ServerDeploy) -> Result<(), OpError> {
        let id = deploy.server_id;
        self.inner.stopping_insert(id);
        for service in &deploy.services {
            self.inner
                .docker
                .kill(&container_name(id, &service.name))
                .await?;
        }
        self.report(id, ServerState::Offline, None);
        Ok(())
    }

    async fn run_delete(&self, server_id: Uuid) -> Result<(), OpError> {
        let docker = &self.inner.docker;
        let dir = self.dir(server_id);
        self.inner.stopping_insert(server_id);
        if let Some(deploy) = self.spec_opt(server_id)? {
            for service in &deploy.services {
                docker
                    .remove_container(&container_name(server_id, &service.name))
                    .await?;
            }
        }
        // Le proxy quitte le réseau du serveur avant que celui-ci soit supprimé.
        self.inner.proxy.remove(server_id).await;
        docker.remove_network(server_id).await?;
        self.inner.specs().remove(&server_id);
        // Des dizaines de Go à effacer : hors des workers du runtime.
        tokio::task::spawn_blocking(move || dir.remove())
            .await
            .map_err(|e| OpError::Spec(e.to_string()))??;
        self.inner.disk().remove(&server_id);
        self.inner.stopping_remove(server_id);
        Ok(())
    }

    /// Fichiers déclarés, écrits conteneurs arrêtés, puis attribués à l'utilisateur du service.
    async fn write_files(&self, deploy: &ServerDeploy) -> Result<(), OpError> {
        let dir = self.dir(deploy.server_id);
        let mut ownership = Ownership::new();
        for file in &deploy.files {
            let Some(written) = dir.write_file(deploy, file)? else {
                continue;
            };
            let service = deploy
                .services
                .iter()
                .find(|s| s.name == file.service)
                .ok_or_else(|| OpError::Spec(format!("unknown service {}", file.service)))?;
            // Volume qui contient le fichier, et chemin relatif à sa racine.
            let Some((volume_dir, relative)) = service.volumes.iter().find_map(|v| {
                let root = dir.volume_dir(&service.name, &v.name);
                let relative = written.strip_prefix(&root).ok()?;
                Some((root.clone(), relative.to_string_lossy().replace('\\', "/")))
            }) else {
                continue;
            };
            let owner = owner_of(deploy, &service.name)?;
            let targets = ownership.entry(owner).or_default();
            match targets.iter_mut().find(|(d, _)| *d == volume_dir) {
                Some((_, paths)) => paths.push(relative),
                None => targets.push((volume_dir, vec![relative])),
            }
        }
        for ((uid, gid), targets) in ownership {
            self.inner.docker.chown(&targets, uid, gid).await?;
        }
        Ok(())
    }

    /// Opération sur le serveur et ses données : refusée pendant une autre, ou une sauvegarde.
    fn acquire(&self, server_id: Uuid) -> Result<BusyGuard, CommandError> {
        let mut busy = self.inner.busy();
        if busy.contains(&server_id) || self.inner.archiving().contains_key(&server_id) {
            return Err(CommandError::Busy);
        }
        busy.insert(server_id);
        Ok(BusyGuard {
            inner: Arc::clone(&self.inner),
            server_id,
        })
    }

    /// Action d'alimentation : permise pendant une sauvegarde.
    fn acquire_power(&self, server_id: Uuid) -> Result<BusyGuard, CommandError> {
        if !self.inner.busy().insert(server_id) {
            return Err(CommandError::Busy);
        }
        Ok(BusyGuard {
            inner: Arc::clone(&self.inner),
            server_id,
        })
    }

    fn is_busy(&self, server_id: Uuid) -> bool {
        self.inner.busy().contains(&server_id)
    }

    fn is_stopping(&self, server_id: Uuid) -> bool {
        self.inner
            .stopping
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&server_id)
    }

    fn report(&self, server_id: Uuid, state: ServerState, reason: Option<String>) {
        self.inner
            .announced()
            .insert(server_id, (state, reason.clone()));
        self.inner.events.send(ServerStateReport {
            server_id,
            state,
            reason,
            web: self.inner.proxy.status(server_id),
        });
    }

    /// Au démarrage : le proxy des sites web reprend les routes des serveurs déployés.
    pub async fn restore_proxy(&self) {
        let mut all = Vec::new();
        for server_id in self.deployed() {
            match self.spec_opt(server_id) {
                Ok(Some(spec)) if !spec.web.is_empty() => all.push((server_id, spec.web)),
                Ok(_) => {}
                Err(error) => tracing::warn!(%server_id, %error, "spec unreadable at boot"),
            }
        }
        self.inner.proxy.restore(all).await;
    }

    /// Relit régulièrement les certificats obtenus par le proxy et annonce les sites dont l'état
    /// change, avec le dernier état connu du serveur.
    pub async fn watch_certificates(&self, every: Duration) {
        let mut ticks = tokio::time::interval(every);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticks.tick().await;
            self.refresh_web_status().await;
        }
    }

    /// Une relecture des certificats : les sites dont l'état change sont annoncés.
    pub async fn refresh_web_status(&self) {
        for server_id in self.inner.proxy.refresh_certificates().await {
            let announced = self.inner.announced().get(&server_id).cloned();
            if let Some((state, reason)) = announced {
                self.report(server_id, state, reason);
            }
        }
    }
}

impl Inner {
    fn busy(&self) -> std::sync::MutexGuard<'_, HashSet<Uuid>> {
        self.busy.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn specs(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, ServerDeploy>> {
        self.specs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn measuring(&self) -> std::sync::MutexGuard<'_, HashSet<Uuid>> {
        self.measuring
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn announced(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, (ServerState, Option<String>)>> {
        self.announced
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn archiving(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, Uuid>> {
        self.archiving
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn uploads(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, upload::Upload>> {
        self.uploads.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn disk(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, u64>> {
        self.disk.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn watches(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, CancellationToken>> {
        self.watches.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn stopping_insert(&self, server_id: Uuid) {
        self.stopping
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(server_id);
    }

    fn stopping_remove(&self, server_id: Uuid) {
        self.stopping
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&server_id);
    }
}

/// Libère le serveur à la fin de l'opération, même en cas d'erreur.
struct BusyGuard {
    inner: Arc<Inner>,
    server_id: Uuid,
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.inner
            .busy
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.server_id);
    }
}

fn owner_of(deploy: &ServerDeploy, service: &str) -> Result<(u32, u32), OpError> {
    let service = deploy
        .services
        .iter()
        .find(|s| s.name == service)
        .ok_or_else(|| OpError::Spec(format!("unknown service {service}")))?;
    spec::check_user(&service.user).map_err(OpError::Spec)
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
