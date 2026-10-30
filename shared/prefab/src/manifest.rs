//! Modèle de `prefab.yaml` (format v1, ADR-0011).
//!
//! Toute clé inconnue est refusée : un service ne peut exprimer que les réglages prévus, jamais
//! `privileged`, un réseau hôte ou un montage de l'hôte. Seules les clés `x-…` (extensions
//! libres) sont acceptées, à la racine et dans un service ; [`crate::validate`] le vérifie.
//!
//! L'ordre des groupes, champs et services est celui du fichier (`IndexMap`) : c'est l'ordre
//! d'affichage dans le panel.

use std::collections::BTreeMap;

use indexmap::IndexMap;
pub use kuro_protocol::api::SteamPlatform;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Valeur attendue de `schema`.
pub const SCHEMA_V1: &str = "kuro.prefab/v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: String,
    pub id: String,
    pub version: semver::Version,
    pub category: Category,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub authors: Vec<Author>,
    #[serde(default)]
    pub requires: Requires,
    #[serde(default)]
    pub assets: Assets,
    pub services: IndexMap<String, Service>,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub files: IndexMap<String, FileDecl>,
    #[serde(default)]
    pub install: Option<Install>,
    #[serde(default)]
    pub backup: Option<Backup>,
    #[serde(default)]
    pub content: IndexMap<String, ContentSlot>,
    pub lifecycle: Lifecycle,
    #[serde(default)]
    pub migrations: Vec<Migration>,
    /// Clés non prévues : seules les extensions `x-…` sont valides.
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Game,
    Web,
    Database,
    Tool,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Author {
    pub name: String,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Requires {
    #[serde(default)]
    pub kuro: Option<String>,
    /// Ressources du serveur entier (ADR-0031). Absent de la sérialisation quand vide : un
    /// manifest stocké avant cet ajout reste identique.
    #[serde(default, skip_serializing_if = "ResourceNeeds::is_empty")]
    pub resources: ResourceNeeds,
}

/// Besoins en ressources déclarés par un prefab (ADR-0031).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceNeeds {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_mb: Option<ResourceNeed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_percent: Option<ResourceNeed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk_mb: Option<ResourceNeed>,
}

/// Minimum (bloquant à chaque changement) et valeur recommandée (pré-remplie).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceNeed {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended: Option<u32>,
}

impl ResourceNeeds {
    pub fn is_empty(&self) -> bool {
        self.memory_mb.is_none() && self.cpu_percent.is_none() && self.disk_mb.is_none()
    }

    /// Chaque ressource déclarée, sous son nom de champ (`memory_mb`…).
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, ResourceNeed)> {
        [
            ("memory_mb", self.memory_mb),
            ("cpu_percent", self.cpu_percent),
            ("disk_mb", self.disk_mb),
        ]
        .into_iter()
        .filter_map(|(name, need)| need.map(|need| (name, need)))
    }

    /// Minimum de la ressource `name`, s'il y en a un.
    pub fn min(&self, name: &str) -> Option<u32> {
        self.iter()
            .find(|(field, _)| *field == name)
            .and_then(|(_, need)| need.min)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assets {
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub cover: Option<String>,
    /// Vignette portrait (jaquette), pour les cartes du catalogue.
    #[serde(default)]
    pub thumbnail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Service {
    pub image: String,
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default)]
    pub env: IndexMap<String, String>,
    #[serde(default)]
    pub ports: IndexMap<String, PortDecl>,
    #[serde(default)]
    pub volumes: IndexMap<String, VolumeDecl>,
    #[serde(default)]
    pub writable: Vec<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub resources: ServiceResources,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub healthcheck: Option<Healthcheck>,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortDecl {
    pub container: u16,
    #[serde(default)]
    pub protocol: Protocol,
    /// Reçoit une allocation sur le nœud.
    #[serde(default)]
    pub public: bool,
    /// Service `SRV` que lit le jeu (`_minecraft._tcp`) : publié vers ce port avec le nom du
    /// serveur (ADR-0026), le joueur n'a alors pas à taper le port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub srv: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    #[default]
    Tcp,
    Udp,
    /// Même numéro en TCP et en UDP (FiveM) : une seule allocation, publiée dans les deux.
    #[serde(rename = "tcp+udp")]
    TcpUdp,
}

impl Protocol {
    /// Protocoles publiés pour ce port.
    pub fn wire(self) -> &'static [kuro_protocol::api::server::PortProtocol] {
        use kuro_protocol::api::server::PortProtocol;
        match self {
            Self::Tcp => &[PortProtocol::Tcp],
            Self::Udp => &[PortProtocol::Udp],
            Self::TcpUdp => &[PortProtocol::Tcp, PortProtocol::Udp],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VolumeDecl {
    pub path: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceResources {
    /// Part (%) des ressources du serveur.
    #[serde(default)]
    pub share: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Healthcheck {
    pub command: Vec<String>,
    #[serde(default = "default_interval")]
    pub interval: u32,
    #[serde(default = "default_retries")]
    pub retries: u32,
}

fn default_interval() -> u32 {
    10
}

fn default_retries() -> u32 {
    3
}

/// Deux sections fixes ; chaque groupe est une sous-page du panel.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(default)]
    pub general: IndexMap<String, Group>,
    #[serde(default)]
    pub advanced: IndexMap<String, Group>,
}

impl Settings {
    /// Tous les champs, dans l'ordre d'affichage, avec leur section et leur groupe.
    pub fn fields(&self) -> impl Iterator<Item = FieldRef<'_>> {
        [
            (Section::General, &self.general),
            (Section::Advanced, &self.advanced),
        ]
        .into_iter()
        .flat_map(|(section, groups)| {
            groups.iter().flat_map(move |(group, def)| {
                def.fields.iter().map(move |(key, field)| FieldRef {
                    section,
                    group,
                    key,
                    field,
                })
            })
        })
    }

    pub fn field(&self, key: &str) -> Option<&Field> {
        self.fields().find(|f| f.key == key).map(|f| f.field)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    General,
    Advanced,
}

#[derive(Debug, Clone, Copy)]
pub struct FieldRef<'a> {
    pub section: Section,
    pub group: &'a str,
    pub key: &'a str,
    pub field: &'a Field,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    /// Nom d'icône Lucide.
    #[serde(default)]
    pub icon: Option<String>,
    pub fields: IndexMap<String, Field>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    #[serde(rename = "type")]
    pub kind: FieldType,
    #[serde(default)]
    pub default: Option<Value>,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub step: Option<f64>,
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub max_length: Option<usize>,
    #[serde(default)]
    pub options: Option<Vec<Value>>,
    #[serde(default)]
    pub visibility: Visibility,
    #[serde(default)]
    pub editable: Editable,
    /// Changer la valeur impose un redémarrage.
    #[serde(default)]
    pub restart: bool,
    /// Secret généré au déploiement.
    #[serde(default)]
    pub generate: Option<Generate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    String,
    Text,
    Int,
    Float,
    Bool,
    Enum,
    Password,
    Secret,
    Slider,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    #[default]
    Admin,
    Customer,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Editable {
    #[default]
    Always,
    /// Fixé au déploiement.
    Deploy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Generate {
    pub length: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileDecl {
    pub service: String,
    pub path: String,
    pub format: FileFormat,
    pub mode: FileMode,
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default)]
    pub keys: IndexMap<String, String>,
    #[serde(default = "default_true")]
    pub editable: bool,
}

fn default_true() -> bool {
    true
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileMode {
    Template,
    ManagedKeys,
    User,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Install {
    pub service: String,
    #[serde(default)]
    pub image: Option<String>,
    pub steps: Vec<Step>,
}

/// Sauvegardes (ADR-0018) : chemins exclus et commandes console autour de l'archivage.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Backup {
    /// Service → chemins du conteneur exclus, sous ses volumes (`*` : un segment, `**` : tous).
    #[serde(default)]
    pub ignore: IndexMap<String, Vec<String>>,
    /// Envoyées au service principal avant l'archivage, serveur en marche.
    #[serde(default)]
    pub before: Vec<BackupHook>,
    /// Envoyées après l'archivage, même s'il a échoué.
    #[serde(default)]
    pub after: Vec<BackupHook>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupHook {
    pub console: String,
    /// Expression régulière attendue sur la console avant de continuer.
    #[serde(default)]
    pub wait: Option<String>,
}

/// Connexion de SteamCMD (ADR-0033).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SteamcmdLogin {
    #[default]
    Anonymous,
    /// Compte Steam de l'installation.
    Account,
}

impl SteamcmdLogin {
    pub fn is_anonymous(&self) -> bool {
        *self == Self::Anonymous
    }
}

impl Manifest {
    /// Une étape `steamcmd` exige le compte Steam de l'installation (ADR-0033).
    pub fn needs_steam_account(&self) -> bool {
        self.install.iter().flat_map(|i| &i.steps).any(|step| {
            matches!(
                step,
                Step::Steamcmd {
                    login: SteamcmdLogin::Account,
                    ..
                }
            )
        })
    }
}

/// Une étape : un seul type par entrée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    Download {
        url: String,
        sha256: String,
        to: String,
    },
    Extract {
        from: String,
        to: String,
        /// Seul ce dossier de l'archive est extrait (`1449/Linux`).
        #[serde(default)]
        subdir: Option<String>,
        /// Chemins relatifs à `to` rendus exécutables (archives sans droits Unix).
        #[serde(default)]
        executable: Vec<String>,
    },
    Copy {
        template: String,
        to: String,
    },
    /// Fichier ou dossier supprimé (archive téléchargée, une fois extraite).
    Remove {
        path: String,
    },
    Steamcmd {
        app: u32,
        dir: String,
        #[serde(default)]
        beta: Option<String>,
        #[serde(default)]
        validate: bool,
        /// `windows` pour un serveur qui n'existe qu'en version Windows (ADR-0032). Absent de la
        /// sérialisation par défaut : un manifest stocké reste identique.
        #[serde(default, skip_serializing_if = "SteamPlatform::is_linux")]
        platform: SteamPlatform,
        /// `account` pour un serveur que SteamCMD refuse en anonyme (Arma 3, ADR-0033) : le compte
        /// Steam de l'installation, réglé dans le panel. Absent de la sérialisation par défaut.
        #[serde(default, skip_serializing_if = "SteamcmdLogin::is_anonymous")]
        login: SteamcmdLogin,
    },
    Script {
        #[serde(default)]
        image: Option<String>,
        run: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentSlot {
    pub kind: ContentKind,
    pub service: String,
    pub providers: Vec<ProviderDecl>,
    pub install: ContentInstall,
    /// Valeurs saisies pour chaque contenu de l'emplacement (ADR-0030), offertes aux gabarits
    /// par `content.<emplacement>.values.<champ>` : les identifiants de mods d'un objet Workshop.
    #[serde(default)]
    pub fields: BTreeMap<String, ContentField>,
    /// Fichiers d'inscription (ADR-0030) : listes JSON tenues par Kuro, une entrée par paquet
    /// actif, réécrites à chaque démarrage (Bedrock : `world_behavior_packs.json`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub register: Vec<ContentRegister>,
}

/// Fichier d'inscription d'un emplacement en mode `extract`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentRegister {
    /// Chemin absolu sous un volume du service ; gabarit (`{{ settings.level_name }}`).
    pub file: String,
    /// Paquets inscrits : ceux dont la description répond ; absent : tous.
    #[serde(default)]
    pub when: Option<ContentWhen>,
    /// Clé → texte fixe, ou `{{ item.<chemin> }}` : la valeur de la description, telle quelle
    /// (texte, nombre, liste).
    pub entry: IndexMap<String, String>,
}

/// Condition sur la description d'un paquet : la valeur au chemin `value` vaut `is`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentWhen {
    /// Chemin dans la description, clés et index séparés par des points (`modules.0.type`).
    pub value: String,
    pub is: String,
}

/// Dossier d'un paquet extrait, choisi d'après sa description.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentRoute {
    pub when: ContentWhen,
    pub dir: String,
}

/// Valeur que l'admin donne à chaque contenu de l'emplacement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentField {
    #[serde(rename = "type")]
    pub kind: ContentFieldType,
    /// Exigée à l'ajout.
    #[serde(default)]
    pub required: bool,
    /// Valeurs proposées d'après la fiche du contenu chez sa source.
    #[serde(default)]
    pub prefill: Option<ContentPrefill>,
}

/// Où chercher les valeurs proposées d'un champ.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentPrefill {
    /// Expression régulière cherchée dans la description, avec un groupe : ce qu'il capture,
    /// découpé sur `,` et `;`, donne les valeurs (`Mod ID: Hydrocraft`).
    pub description: String,
}

/// Valeurs proposées au plus, et caractères d'une valeur (comme une saisie, ADR-0030).
const MAX_PREFILL_VALUES: usize = 32;
const MAX_PREFILL_CHARS: usize = 128;

impl ContentPrefill {
    /// Motif compilé, borné en taille : un prefab ne peut pas bloquer l'API (moteur linéaire).
    pub fn pattern(&self) -> Result<regex::Regex, regex::Error> {
        regex::RegexBuilder::new(&self.description)
            .size_limit(1 << 20)
            .build()
    }

    /// Valeurs trouvées dans une description : le premier groupe de chaque correspondance,
    /// découpé sur `,` et `;`, sans doublon ni valeur invalide.
    pub fn values(&self, description: &str) -> Vec<String> {
        let Ok(pattern) = self.pattern() else {
            return Vec::new();
        };
        let mut values: Vec<String> = Vec::new();
        for captures in pattern.captures_iter(description) {
            let Some(found) = captures.get(1) else {
                continue;
            };
            for value in found.as_str().split([',', ';']).map(str::trim) {
                let valid = !value.is_empty()
                    && value.chars().count() <= MAX_PREFILL_CHARS
                    && !value.chars().any(char::is_control);
                if valid && !values.iter().any(|v| v == value) {
                    values.push(value.to_owned());
                }
            }
        }
        values.truncate(MAX_PREFILL_VALUES);
        values
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentFieldType {
    /// Une valeur.
    String,
    /// Plusieurs valeurs (un objet Workshop peut contenir plusieurs mods).
    List,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentKind {
    Mods,
    Modpacks,
    Resourcepacks,
    Plugins,
    Maps,
    /// Add-ons Bedrock (packs de comportement et de ressources).
    Addons,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderDecl {
    pub provider: Provider,
    #[serde(default)]
    pub game: Option<String>,
    #[serde(default)]
    pub class: Option<String>,
    #[serde(default)]
    pub loader: Option<String>,
    /// Version du jeu, gabarit (`{{ settings.version }}`) : filtre les versions compatibles
    /// (ADR-0020).
    #[serde(default)]
    pub game_version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Provider {
    Curseforge,
    Modrinth,
    SteamWorkshop,
    Upload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentInstall {
    pub mode: ContentInstallMode,
    /// Dossier de dépôt ; en mode `extract`, celui des archives téléchargées (le dossier de la
    /// première route par défaut).
    #[serde(default)]
    pub dir: Option<String>,
    #[serde(default)]
    pub load_order: Option<String>,
    /// Mode `extract` : fichier de description de chaque paquet (`manifest.json`).
    #[serde(default)]
    pub describe: Option<String>,
    /// Mode `extract` : dossier de chaque paquet, d'après sa description.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub route: Vec<ContentRoute>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentInstallMode {
    Download,
    Reference,
    /// Archive extraite en paquets, chacun rangé d'après sa description (ADR-0030).
    Extract,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lifecycle {
    pub service: String,
    #[serde(default)]
    pub ready: Option<Ready>,
    pub stop: Stop,
    #[serde(default = "default_timeout")]
    pub timeout: u32,
}

fn default_timeout() -> u32 {
    30
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Ready {
    Console(String),
    /// `healthcheck: true`.
    Healthcheck(bool),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Stop {
    Console(String),
    Signal(Signal),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Signal {
    #[serde(rename = "SIGTERM")]
    Term,
    #[serde(rename = "SIGINT")]
    Int,
    #[serde(rename = "SIGQUIT")]
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Migration {
    pub from: String,
    #[serde(default)]
    pub rename: BTreeMap<String, String>,
    #[serde(default)]
    pub remove: Vec<String>,
}
