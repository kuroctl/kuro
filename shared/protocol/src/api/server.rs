//! Commandes de l'API sur les serveurs d'un nœud (ADR-0013).
//!
//! Le daemon ne connaît pas les prefabs : `server.deploy` lui transmet une spécification déjà
//! résolue (images, variables, ports, fichiers). Il l'applique avec ses propres règles de
//! sécurité, sans faire confiance à l'API sur ce point.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Création ou mise à jour d'un serveur : le daemon remplace ce qui a changé.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerDeploy {
    pub server_id: Uuid,
    pub services: Vec<ServiceSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<FileSpec>,
    pub lifecycle: LifecycleSpec,
    /// Étapes d'installation (ADR-0017), exécutées juste après la création des conteneurs, sur
    /// les volumes existants : à la création du serveur, ou lors d'une mise à jour du prefab qui
    /// les change (ADR-0024). Un redéploiement après modification des réglages n'en porte pas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install: Option<crate::api::install::InstallSpec>,
    /// Quota disque (volumes et sauvegardes, ADR-0019) ; `0` : illimité.
    #[serde(default)]
    pub disk_bytes: u64,
    /// Serveur suspendu (ADR-0019) : un redéploiement garde la suspension.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub suspended: bool,
    /// Serveur en marche : ne pas l'arrêter. La spécification est gardée et appliquée au
    /// prochain démarrage (changement de contenu, ADR-0030) ; serveur arrêté : sans effet.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub defer: bool,
}

/// Un conteneur du serveur.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceSpec {
    /// Nom du service, aussi son nom sur le réseau privé du serveur.
    pub name: String,
    pub image: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<PortSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub volumes: Vec<VolumeSpec>,
    /// Chemins montés en `tmpfs` ; le reste de la racine est en lecture seule.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub writable: Vec<String>,
    /// `uid:gid`, jamais root.
    pub user: String,
    pub limits: Limits,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub healthcheck: Option<HealthcheckSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortSpec {
    pub name: String,
    pub container_port: u16,
    pub protocol: PortProtocol,
    /// Allocation du nœud ; absent pour un port interne au réseau du serveur.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published: Option<PublishedPort>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortProtocol {
    Tcp,
    Udp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedPort {
    pub host_ip: String,
    pub host_port: u16,
}

/// Volume persistant ; le daemon choisit son emplacement sur l'hôte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeSpec {
    pub name: String,
    /// Chemin absolu dans le conteneur.
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    pub memory_bytes: u64,
    /// 1000 = un cœur.
    pub cpu_millis: u32,
    pub pids: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthcheckSpec {
    pub command: Vec<String>,
    pub interval_secs: u32,
    pub retries: u32,
}

/// Fichier écrit par le daemon avant chaque démarrage, sous un volume du service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileSpec {
    pub service: String,
    /// Chemin absolu dans le conteneur.
    pub path: String,
    pub format: FileFormat,
    #[serde(flatten)]
    pub content: FileContent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "kebab-case")]
pub enum FileContent {
    /// Remplacé à chaque démarrage.
    Template { content: String },
    /// Seules ces clés sont réécrites ; le reste du fichier est conservé.
    ManagedKeys { keys: BTreeMap<String, String> },
    /// Créé s'il n'existe pas, jamais modifié ensuite.
    User {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileFormat {
    Ini,
    Properties,
    Json,
    Yaml,
    Toml,
    Xml,
    Cfg,
    Lua,
    Php,
    Sh,
    Text,
}

/// Démarrage et arrêt du serveur, portés par son service principal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleSpec {
    pub service: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready: Option<ReadySpec>,
    pub stop: StopSpec,
    pub stop_timeout_secs: u32,
}

/// Passage de `starting` à `running`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadySpec {
    /// Expression régulière attendue sur la console du service principal.
    Console(String),
    /// Healthcheck du service principal en bonne santé.
    Healthcheck,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopSpec {
    /// Commande envoyée sur l'entrée de la console.
    Console(String),
    Signal(StopSignal),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopSignal {
    #[serde(rename = "SIGTERM")]
    Term,
    #[serde(rename = "SIGINT")]
    Int,
    #[serde(rename = "SIGQUIT")]
    Quit,
}

impl StopSignal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Term => "SIGTERM",
            Self::Int => "SIGINT",
            Self::Quit => "SIGQUIT",
        }
    }
}

/// Arrête le serveur, supprime ses conteneurs, son réseau et ses données.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerDelete {
    pub server_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerPower {
    pub server_id: Uuid,
    pub action: PowerAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerAction {
    Start,
    Stop,
    Restart,
    Kill,
}

/// Serveurs assignés au nœud, envoyé après `hello.ack` : le daemon renvoie leur état réel et
/// signale les conteneurs Kuro inconnus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerSync {
    pub server_ids: Vec<Uuid>,
}

/// Abonnement à la console et aux stats d'un serveur (ADR-0014) : le daemon ne les lit que
/// pendant un `watch: true`. Tous les abonnements tombent avec le lien.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerWatch {
    pub server_id: Uuid,
    pub watch: bool,
}

/// Ligne à écrire sur l'entrée de la console du service principal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerCommand {
    pub server_id: Uuid,
    /// Une seule ligne, sans caractère de contrôle (voir [`crate::limits::is_valid_command`]).
    pub line: String,
}

/// Nouvelles ressources d'un serveur (ADR-0019), appliquées à chaud : limites par service, déjà
/// réparties comme au déploiement, et quota disque (`0` : illimité).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerLimits {
    pub server_id: Uuid,
    pub services: BTreeMap<String, Limits>,
    pub disk_bytes: u64,
}

/// Suspend ou réactive un serveur (ADR-0019). Suspendre l'arrête.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerSuspend {
    pub server_id: Uuid,
    pub suspended: bool,
}
