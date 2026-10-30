//! Import d'un `docker-compose.yml` dans le Studio (ADR-0011) : conversion de ce qui est sûr en
//! `prefab.yaml`, refus explicite du reste.
//!
//! Le format des prefabs est une liste blanche : rien de ce qui ouvre l'hôte (`privileged`,
//! réseau ou PID de l'hôte, `cap_add`, devices, montage d'un dossier de l'hôte, root) n'a
//! d'équivalent. La conversion ne cherche pas à contourner ces refus : elle les nomme, et le
//! manifest produit passe ensuite par la validation habituelle.
//!
//! Les variables `${NOM}` / `${NOM:-défaut}` deviennent des réglages (`{{ settings.nom }}`) ;
//! un nom de mot de passe, de jeton ou de clé devient un secret généré au déploiement.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::LazyLock;

use indexmap::IndexMap;
use regex::Regex;
use serde_json::Value;

static VARIABLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)(?::?-([^}]*))?\}|\$([A-Za-z_][A-Za-z0-9_]*)|\$\$")
        .unwrap_or_else(|_| unreachable!("static regex"))
});
static DURATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(\d+)(ms|s|m|h)").unwrap_or_else(|_| unreachable!("static regex"))
});
static NUMERIC_USER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[1-9][0-9]*(:[0-9]+)?$").unwrap_or_else(|_| unreachable!("static regex"))
});

/// Taille maximale du `docker-compose.yml` accepté.
pub const MAX_COMPOSE_BYTES: usize = 256 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ComposeError {
    #[error("docker-compose file larger than {MAX_COMPOSE_BYTES} bytes")]
    TooLarge,
    #[error("unreadable docker-compose file: {0}")]
    Unreadable(String),
    #[error("no service to convert")]
    NoService,
}

/// Ce que la conversion a fait d'une clé du compose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoteKind {
    /// Convertie, avec une adaptation à connaître.
    Adapted,
    /// Sans effet dans Kuro (géré autrement) : ignorée.
    Ignored,
    /// Contraire au durcissement ou sans équivalent : refusée.
    Refused,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    /// Emplacement dans le compose (`services.db.privileged`).
    pub path: String,
    pub kind: NoteKind,
    pub message: String,
}

/// Résultat : fichiers du futur brouillon et notes de conversion.
#[derive(Debug, Clone)]
pub struct Conversion {
    pub id: String,
    /// Chemin relatif → contenu (`prefab.yaml`, `locales/fr.json`, `locales/en.json`).
    pub files: BTreeMap<String, String>,
    pub notes: Vec<Note>,
}

#[derive(Debug, Default)]
struct Port {
    name: String,
    container: u16,
    udp: bool,
    public: bool,
}

#[derive(Debug, Default)]
struct Service {
    name: String,
    image: String,
    command: Vec<String>,
    env: Vec<(String, String)>,
    ports: Vec<Port>,
    volumes: Vec<(String, String)>,
    writable: Vec<String>,
    user: Option<String>,
    depends_on: Vec<String>,
    healthcheck: Option<(Vec<String>, Option<u32>, Option<u32>)>,
}

#[derive(Debug)]
struct Setting {
    key: String,
    /// Nom de la variable dans le compose, libellé par défaut du réglage.
    variable: String,
    secret: bool,
    default: Option<String>,
}

struct Converter {
    notes: Vec<Note>,
    settings: IndexMap<String, Setting>,
}

const IGNORED: &[(&str, &str)] = &[
    (
        "restart",
        "Kuro relance le serveur selon ses actions d'alimentation",
    ),
    ("container_name", "les conteneurs sont nommés par Kuro"),
    (
        "hostname",
        "les services se joignent par leur nom sur le réseau privé du serveur",
    ),
    (
        "networks",
        "les services d'un serveur partagent un réseau privé",
    ),
    (
        "expose",
        "tous les ports sont joignables sur le réseau privé du serveur",
    ),
    ("labels", "les étiquettes sont posées par Kuro"),
    ("logging", "la console est relayée par Kuro"),
    (
        "stop_grace_period",
        "le délai d'arrêt est `lifecycle.timeout`",
    ),
    ("stop_signal", "le signal d'arrêt est `lifecycle.stop`"),
    ("deploy", "les ressources sont réglées par serveur"),
    ("mem_limit", "les ressources sont réglées par serveur"),
    ("memswap_limit", "les ressources sont réglées par serveur"),
    ("cpus", "les ressources sont réglées par serveur"),
    ("cpu_shares", "les ressources sont réglées par serveur"),
    ("pids_limit", "les ressources sont réglées par serveur"),
    ("shm_size", "les ressources sont réglées par serveur"),
    ("tty", "la console est relayée par Kuro"),
    ("stdin_open", "la console est relayée par Kuro"),
    ("working_dir", "le dossier de travail est celui de l'image"),
    ("init", "Kuro démarre le processus de l'image"),
    ("platform", "l'architecture est celle du nœud"),
    ("pull_policy", "l'image est téléchargée par le daemon"),
];

const REFUSED: &[(&str, &str)] = &[
    ("privileged", "un conteneur privilégié ouvre l'hôte"),
    (
        "cap_add",
        "les capacités sont toutes retirées (`cap_drop: ALL`)",
    ),
    ("cap_drop", "les capacités sont toutes retirées par Kuro"),
    ("devices", "aucun périphérique de l'hôte n'est exposé"),
    (
        "device_cgroup_rules",
        "aucun périphérique de l'hôte n'est exposé",
    ),
    ("network_mode", "le réseau de l'hôte n'est jamais partagé"),
    (
        "pid",
        "l'espace de processus de l'hôte n'est jamais partagé",
    ),
    ("ipc", "l'IPC de l'hôte n'est jamais partagé"),
    ("uts", "l'espace de noms de l'hôte n'est jamais partagé"),
    ("userns_mode", "l'espace utilisateur est fixé par Kuro"),
    ("cgroup_parent", "les cgroups sont fixés par Kuro"),
    ("security_opt", "le profil de sécurité est fixé par Kuro"),
    ("sysctls", "les paramètres noyau ne sont pas modifiables"),
    ("ulimits", "les limites sont fixées par Kuro"),
    (
        "extra_hosts",
        "la résolution de noms est celle du réseau privé",
    ),
    ("dns", "la résolution de noms est celle du réseau privé"),
    (
        "dns_search",
        "la résolution de noms est celle du réseau privé",
    ),
    ("links", "les services se joignent par leur nom, sans lien"),
    (
        "external_links",
        "un serveur ne joint pas les conteneurs d'un autre",
    ),
    ("volumes_from", "les volumes sont propres à chaque service"),
    ("runtime", "le runtime est fixé par le nœud"),
    (
        "env_file",
        "les fichiers de l'hôte ne sont pas lus : déclarez les variables",
    ),
    ("secrets", "utilisez un réglage de type secret"),
    ("configs", "utilisez un fichier géré (`files`)"),
    (
        "build",
        "Kuro ne construit pas d'image : publiez-la dans un registre",
    ),
    ("profiles", "tous les services d'un prefab sont lancés"),
    ("extends", "décrivez le service en entier"),
];

pub(crate) fn slug(text: &str, separator: char) -> String {
    let mut out = String::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with(separator) {
            out.push(separator);
        }
    }
    let out = out.trim_end_matches(separator).to_owned();
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        format!("x{separator}{out}")
    } else {
        out
    }
}

/// Chaîne YAML sûre (JSON est du YAML valide).
pub(crate) fn q(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_owned())
}

fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

/// Découpage d'une commande écrite en une chaîne (`sh -c "…"` gardé entre guillemets).
fn split_command(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for c in text.chars() {
        match (quote, c) {
            (Some(open), _) if c == open => quote = None,
            (Some(_), _) => current.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started || !current.is_empty() {
                    parts.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            (None, c) => current.push(c),
        }
    }
    if started || !current.is_empty() {
        parts.push(current);
    }
    parts
}

/// `10s`, `1m30s`, `500ms` → secondes (au moins 1).
fn duration(text: &str) -> Option<u32> {
    let mut total = 0u64;
    let mut matched = false;
    for capture in DURATION.captures_iter(text) {
        matched = true;
        let amount: u64 = capture[1].parse().ok()?;
        total += match &capture[2] {
            "ms" => amount / 1000,
            "s" => amount,
            "m" => amount * 60,
            _ => amount * 3600,
        };
    }
    matched.then(|| u32::try_from(total.max(1)).unwrap_or(u32::MAX))
}

/// `mysql:8` → `docker.io/library/mysql:8` ; un registre explicite est gardé.
fn normalize_image(image: &str) -> String {
    let first = image.split('/').next().unwrap_or_default();
    let has_registry =
        image.contains('/') && (first.contains('.') || first.contains(':') || first == "localhost");
    if has_registry {
        image.to_owned()
    } else if image.contains('/') {
        format!("docker.io/{image}")
    } else {
        format!("docker.io/library/{image}")
    }
}

impl Converter {
    fn note(&mut self, path: impl Into<String>, kind: NoteKind, message: impl Into<String>) {
        self.notes.push(Note {
            path: path.into(),
            kind,
            message: message.into(),
        });
    }

    /// Remplace les variables du compose par des réglages.
    fn template(&mut self, value: &str) -> String {
        VARIABLE
            .replace_all(value, |capture: &regex::Captures<'_>| {
                if &capture[0] == "$$" {
                    return "$".to_owned();
                }
                let name = capture
                    .get(1)
                    .or_else(|| capture.get(3))
                    .map_or("", |m| m.as_str());
                let key = slug(name, '_');
                let upper = name.to_ascii_uppercase();
                let secret = ["PASSWORD", "PASSWD", "SECRET", "TOKEN", "KEY"]
                    .iter()
                    .any(|word| upper.contains(word));
                let default = capture.get(2).map(|m| m.as_str().to_owned());
                self.settings.entry(key.clone()).or_insert(Setting {
                    key: key.clone(),
                    variable: name.to_owned(),
                    secret: secret && default.is_none(),
                    default,
                });
                format!("{{{{ settings.{key} }}}}")
            })
            .into_owned()
    }

    fn service(
        &mut self,
        name: &str,
        spec: &IndexMap<String, Value>,
        named_volumes: &[String],
    ) -> Option<Service> {
        let at = |key: &str| format!("services.{name}.{key}");
        let mut service = Service {
            name: slug(name, '-'),
            writable: vec!["/tmp".to_owned()],
            ..Service::default()
        };
        if service.name != name {
            self.note(
                format!("services.{name}"),
                NoteKind::Adapted,
                format!("renommé `{}`", service.name),
            );
        }
        let mut entrypoint = Vec::new();
        for (key, value) in spec {
            match key.as_str() {
                "image" => match scalar(value) {
                    Some(image) => {
                        service.image = normalize_image(&image);
                        let tagged = service
                            .image
                            .rsplit('/')
                            .next()
                            .is_some_and(|last| last.contains(':') || last.contains('@'));
                        if !tagged || service.image.ends_with(":latest") {
                            self.note(at("image"), NoteKind::Adapted, "image sans version datée : épinglez un tag précis (jamais `latest`)");
                        } else if service.image != image {
                            self.note(
                                at("image"),
                                NoteKind::Adapted,
                                format!("registre explicite : `{}`", service.image),
                            );
                        }
                    }
                    None => self.note(at("image"), NoteKind::Refused, "image illisible"),
                },
                "command" | "entrypoint" => {
                    let parts = match value {
                        Value::String(text) => split_command(text),
                        Value::Array(items) => items.iter().filter_map(scalar).collect(),
                        _ => Vec::new(),
                    };
                    let parts: Vec<String> = parts.iter().map(|part| self.template(part)).collect();
                    if key == "entrypoint" {
                        // Kuro garde le point d'entrée de l'image et passe `command` en
                        // arguments (ADR-0039) : un `entrypoint` du compose ne peut pas être
                        // reproduit. Dit plutôt que fusionné en silence.
                        self.note(
                            at("entrypoint"),
                            NoteKind::Refused,
                            format!(
                                "point d'entrée de l'image conservé : `{}` non repris, vérifiez `command`",
                                parts.join(" ")
                            ),
                        );
                    } else {
                        service.command = parts;
                    }
                }
                "environment" => {
                    let pairs: Vec<(String, Option<String>)> = match value {
                        Value::Object(map) => {
                            map.iter().map(|(k, v)| (k.clone(), scalar(v))).collect()
                        }
                        Value::Array(items) => items
                            .iter()
                            .filter_map(scalar)
                            .map(|item| match item.split_once('=') {
                                Some((k, v)) => (k.to_owned(), Some(v.to_owned())),
                                None => (item, None),
                            })
                            .collect(),
                        _ => Vec::new(),
                    };
                    for (variable, value) in pairs {
                        let value = match value {
                            Some(value) => self.template(&value),
                            // `- NOM` sans valeur : lu de l'environnement de l'hôte dans compose.
                            None => self.template(&format!("${{{variable}}}")),
                        };
                        service.env.push((variable, value));
                    }
                }
                "ports" => {
                    for (index, port) in value.as_array().into_iter().flatten().enumerate() {
                        let path = format!("{}[{index}]", at("ports"));
                        match self.port(&path, port) {
                            Ok(mut port) => {
                                let base = format!(
                                    "{}{}",
                                    if port.udp { "udp" } else { "p" },
                                    port.container
                                );
                                port.name = base;
                                if service.ports.iter().any(|other| other.name == port.name) {
                                    continue;
                                }
                                service.ports.push(port);
                            }
                            Err(reason) => self.note(path, NoteKind::Refused, reason),
                        }
                    }
                }
                "volumes" => {
                    for (index, volume) in value.as_array().into_iter().flatten().enumerate() {
                        let path = format!("{}[{index}]", at("volumes"));
                        self.volume(&path, volume, named_volumes, &mut service);
                    }
                }
                "tmpfs" => {
                    let paths: Vec<String> = match value {
                        Value::String(text) => vec![text.clone()],
                        Value::Array(items) => items.iter().filter_map(scalar).collect(),
                        _ => Vec::new(),
                    };
                    for path in paths {
                        let path = path.split(':').next().unwrap_or_default().to_owned();
                        if !service.writable.contains(&path) {
                            service.writable.push(path);
                        }
                    }
                    self.note(at("tmpfs"), NoteKind::Adapted, "converti en `writable`");
                }
                "user" => {
                    let user = scalar(value).unwrap_or_default();
                    if NUMERIC_USER.is_match(&user) {
                        service.user = Some(if user.contains(':') {
                            user
                        } else {
                            format!("{user}:{user}")
                        });
                    } else {
                        self.note(at("user"), NoteKind::Refused, "root et les noms d'utilisateur sont refusés : indiquez un uid:gid non nul");
                    }
                }
                "depends_on" => {
                    service.depends_on = match value {
                        Value::Array(items) => items.iter().filter_map(scalar).collect(),
                        Value::Object(map) => map.keys().cloned().collect(),
                        _ => Vec::new(),
                    }
                    .iter()
                    .map(|name| slug(name, '-'))
                    .collect();
                }
                "healthcheck" => self.healthcheck(&at("healthcheck"), value, &mut service),
                other if other.starts_with("x-") => {}
                other => {
                    if let Some((_, reason)) = REFUSED.iter().find(|(name, _)| *name == other) {
                        self.note(at(other), NoteKind::Refused, *reason);
                    } else if let Some((_, reason)) =
                        IGNORED.iter().find(|(name, _)| *name == other)
                    {
                        self.note(at(other), NoteKind::Ignored, *reason);
                    } else {
                        self.note(
                            at(other),
                            NoteKind::Refused,
                            "clé sans équivalent dans un prefab",
                        );
                    }
                }
            }
        }
        if service.image.is_empty() {
            self.note(
                format!("services.{name}"),
                NoteKind::Refused,
                "service sans image : non converti",
            );
            return None;
        }
        if !entrypoint.is_empty() {
            entrypoint.extend(std::mem::take(&mut service.command));
            service.command = entrypoint;
        }
        Some(service)
    }

    fn port(&mut self, path: &str, value: &Value) -> Result<Port, String> {
        let (published, target, protocol) = match value {
            Value::Object(map) => (
                map.get("published").and_then(scalar),
                map.get("target").and_then(scalar).unwrap_or_default(),
                map.get("protocol")
                    .and_then(scalar)
                    .unwrap_or_else(|| "tcp".to_owned()),
            ),
            other => {
                let text = scalar(other).unwrap_or_default();
                let (spec, protocol) = text.split_once('/').unwrap_or((&text, "tcp"));
                let parts: Vec<&str> = spec.rsplitn(3, ':').collect();
                (
                    parts.get(1).map(|p| (*p).to_owned()),
                    parts.first().copied().unwrap_or_default().to_owned(),
                    protocol.to_owned(),
                )
            }
        };
        if target.contains('-') {
            return Err("plage de ports non prise en charge : déclarez chaque port".to_owned());
        }
        let container: u16 = target
            .parse()
            .map_err(|_| format!("port illisible : `{target}`"))?;
        if container == 0 {
            return Err("port 0 refusé".to_owned());
        }
        if published.as_deref().is_some_and(|host| host != target) {
            self.note(
                path,
                NoteKind::Adapted,
                "le port de l'hôte est attribué par Kuro (allocation du nœud)",
            );
        }
        Ok(Port {
            name: String::new(),
            container,
            udp: protocol == "udp",
            public: true,
        })
    }

    fn volume(&mut self, path: &str, value: &Value, named: &[String], service: &mut Service) {
        let (source, target, kind) = match value {
            Value::Object(map) => (
                map.get("source").and_then(scalar),
                map.get("target").and_then(scalar).unwrap_or_default(),
                map.get("type")
                    .and_then(scalar)
                    .unwrap_or_else(|| "volume".to_owned()),
            ),
            other => {
                let text = scalar(other).unwrap_or_default();
                let parts: Vec<&str> = text.split(':').collect();
                match parts.as_slice() {
                    [target] => (None, (*target).to_owned(), "volume".to_owned()),
                    [source, target, ..] => {
                        let bind = source.starts_with('.')
                            || source.starts_with('/')
                            || source.starts_with('~');
                        (
                            Some((*source).to_owned()),
                            (*target).to_owned(),
                            if bind { "bind" } else { "volume" }.to_owned(),
                        )
                    }
                    [] => (None, String::new(), "volume".to_owned()),
                }
            }
        };
        if !target.starts_with('/') {
            self.note(path, NoteKind::Refused, "chemin du conteneur illisible");
            return;
        }
        match kind.as_str() {
            "tmpfs" => service.writable.push(target),
            "bind" => {
                let name = slug(
                    source
                        .as_deref()
                        .and_then(|s| s.rsplit('/').find(|part| !part.is_empty() && *part != "."))
                        .unwrap_or("data"),
                    '_',
                );
                self.note(
                    path,
                    NoteKind::Adapted,
                    format!("montage de l'hôte refusé : remplacé par le volume `{name}`"),
                );
                service.volumes.push((name, target));
            }
            _ => {
                let name = match &source {
                    Some(source) => {
                        if !named.contains(source) {
                            self.note(
                                path,
                                NoteKind::Adapted,
                                format!(
                                    "volume `{source}` absent de `volumes` : créé pour ce service"
                                ),
                            );
                        }
                        slug(source, '_')
                    }
                    None => slug(
                        target
                            .rsplit('/')
                            .find(|part| !part.is_empty())
                            .unwrap_or("data"),
                        '_',
                    ),
                };
                service.volumes.push((name, target));
            }
        }
    }

    fn healthcheck(&mut self, path: &str, value: &Value, service: &mut Service) {
        let Some(map) = value.as_object() else { return };
        if map.get("disable").and_then(Value::as_bool) == Some(true) {
            return;
        }
        let command = match map.get("test") {
            Some(Value::String(text)) => vec!["sh".to_owned(), "-c".to_owned(), text.clone()],
            Some(Value::Array(items)) => {
                let items: Vec<String> = items.iter().filter_map(scalar).collect();
                match items.split_first() {
                    Some((first, rest)) if first == "CMD" => rest.to_vec(),
                    Some((first, rest)) if first == "CMD-SHELL" => {
                        vec!["sh".to_owned(), "-c".to_owned(), rest.join(" ")]
                    }
                    Some((first, _)) if first == "NONE" => return,
                    _ => items,
                }
            }
            _ => return,
        };
        if command.first().is_some_and(|first| first == "sh") {
            self.note(path, NoteKind::Adapted, "commande shell : `sh -c`");
        }
        let interval = map
            .get("interval")
            .and_then(scalar)
            .and_then(|text| duration(&text));
        let retries = map
            .get("retries")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok());
        service.healthcheck = Some((command, interval, retries));
    }
}

/// Services du compose, dans l'ordre du fichier (le reste est relu en `Value`).
#[derive(serde::Deserialize)]
struct ServicesOnly {
    #[serde(default)]
    services: IndexMap<String, IndexMap<String, Value>>,
}

/// Convertit un `docker-compose.yml`. `id` : identifiant voulu (sinon `name:` du compose, ou le
/// premier service).
pub fn convert(text: &str, id: Option<&str>) -> Result<Conversion, ComposeError> {
    if text.len() > MAX_COMPOSE_BYTES {
        return Err(ComposeError::TooLarge);
    }
    let root: IndexMap<String, Value> = serde_saphyr::from_str(text)
        .map_err(|error| ComposeError::Unreadable(error.to_string()))?;
    // Relus à part, dans l'ordre du fichier : `Value` trie ses clés, et le premier service
    // donne l'identifiant et le service principal.
    let services = match root.get("services") {
        Some(_) => {
            serde_saphyr::from_str::<ServicesOnly>(text)
                .map_err(|error| ComposeError::Unreadable(error.to_string()))?
                .services
        }
        None => IndexMap::new(),
    };
    if services.is_empty() {
        return Err(ComposeError::NoService);
    }
    let named_volumes: Vec<String> = root
        .get("volumes")
        .and_then(Value::as_object)
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default();

    let mut converter = Converter {
        notes: Vec::new(),
        settings: IndexMap::new(),
    };
    for (key, _) in &root {
        match key.as_str() {
            "services" | "volumes" => {}
            "version" | "name" => {}
            "networks" => converter.note(
                "networks",
                NoteKind::Ignored,
                "les services d'un serveur partagent un réseau privé",
            ),
            "secrets" | "configs" => converter.note(
                key.clone(),
                NoteKind::Refused,
                "utilisez un réglage secret ou un fichier géré",
            ),
            other if other.starts_with("x-") => {}
            other => converter.note(
                other.to_owned(),
                NoteKind::Refused,
                "clé sans équivalent dans un prefab",
            ),
        }
    }
    let converted: Vec<Service> = services
        .iter()
        .filter_map(|(name, spec)| converter.service(name, spec, &named_volumes))
        .collect();
    if converted.is_empty() {
        return Err(ComposeError::NoService);
    }

    let id = slug(
        id.or_else(|| root.get("name").and_then(Value::as_str))
            .unwrap_or(&converted[0].name),
        '-',
    );
    let id = if id.is_empty() {
        "imported".to_owned()
    } else {
        id
    };
    let main = converted
        .iter()
        .find(|service| service.ports.iter().any(|port| port.public))
        .unwrap_or(&converted[0]);
    let web = converted
        .iter()
        .flat_map(|service| &service.ports)
        .any(|port| [80, 443, 8080, 8000, 3000].contains(&port.container));

    let manifest = write_manifest(&id, web, &converted, main, &converter);
    let title = id.replace('-', " ");
    let title = title[..1].to_uppercase() + &title[1..];
    let fields: serde_json::Map<String, Value> = converter
        .settings
        .values()
        .map(|setting| {
            (
                setting.key.clone(),
                serde_json::json!({ "label": setting.variable }),
            )
        })
        .collect();
    let locale = |lang: &str| {
        let description = if lang == "fr" {
            "Importé depuis un docker-compose."
        } else {
            "Imported from a docker-compose file."
        };
        format!(
            "{}\n",
            serde_json::to_string_pretty(&serde_json::json!({
                "prefab": { "name": title, "description": description },
                "groups": { "config": { "title": "Configuration" }, "secrets": { "title": "Secrets" } },
                "fields": fields,
            }))
            .unwrap_or_default()
        )
    };
    let files = BTreeMap::from([
        ("prefab.yaml".to_owned(), manifest),
        ("locales/fr.json".to_owned(), locale("fr")),
        ("locales/en.json".to_owned(), locale("en")),
    ]);
    Ok(Conversion {
        id,
        files,
        notes: converter.notes,
    })
}

fn write_manifest(
    id: &str,
    web: bool,
    services: &[Service],
    main: &Service,
    converter: &Converter,
) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# Importé depuis un docker-compose (Prefab Studio). À relire avant publication :"
    );
    let _ = writeln!(
        out,
        "# images épinglées, utilisateur non-root (`user`), dossiers inscriptibles (`writable`)."
    );
    for note in converter
        .notes
        .iter()
        .filter(|note| note.kind == NoteKind::Refused)
    {
        let _ = writeln!(out, "# refusé : {} : {}", note.path, note.message);
    }
    let _ = writeln!(out, "schema: kuro.prefab/v1");
    let _ = writeln!(out, "id: {id}");
    let _ = writeln!(out, "version: 0.1.0");
    let _ = writeln!(out, "category: {}", if web { "web" } else { "other" });
    let _ = writeln!(out, "tags: []");
    let _ = writeln!(out, "requires:\n  kuro: \">=0.2\"\n");
    let _ = writeln!(out, "services:");
    let shares = shares(services.len());
    for (service, share) in services.iter().zip(shares) {
        let _ = writeln!(out, "  {}:", service.name);
        let _ = writeln!(out, "    image: {}", q(&service.image));
        if !service.command.is_empty() {
            let parts: Vec<String> = service.command.iter().map(|part| q(part)).collect();
            let _ = writeln!(out, "    command: [{}]", parts.join(", "));
        }
        if !service.env.is_empty() {
            let _ = writeln!(out, "    env:");
            for (name, value) in &service.env {
                let _ = writeln!(out, "      {name}: {}", q(value));
            }
        }
        if !service.ports.is_empty() {
            let _ = writeln!(out, "    ports:");
            for port in &service.ports {
                let _ = writeln!(
                    out,
                    "      {}: {{ container: {}, protocol: {}, public: {} }}",
                    port.name,
                    port.container,
                    if port.udp { "udp" } else { "tcp" },
                    port.public
                );
            }
        }
        if !service.volumes.is_empty() {
            let _ = writeln!(out, "    volumes:");
            let mut seen = Vec::new();
            for (name, path) in &service.volumes {
                let mut key = name.clone();
                let mut index = 2;
                while seen.contains(&key) {
                    key = format!("{name}_{index}");
                    index += 1;
                }
                seen.push(key.clone());
                let _ = writeln!(out, "      {key}: {{ path: {} }}", q(path));
            }
        }
        let writable: Vec<String> = service.writable.iter().map(|path| q(path)).collect();
        let _ = writeln!(out, "    writable: [{}]", writable.join(", "));
        match &service.user {
            Some(user) => {
                let _ = writeln!(out, "    user: {}", q(user));
            }
            None => {
                let _ = writeln!(
                    out,
                    "    # user: \"1000:1000\"  # uid:gid non nul de l'image, à renseigner"
                );
            }
        }
        if !service.depends_on.is_empty() {
            let _ = writeln!(out, "    depends_on: [{}]", service.depends_on.join(", "));
        }
        if let Some(share) = share {
            let _ = writeln!(out, "    resources: {{ share: {share} }}");
        }
        if let Some((command, interval, retries)) = &service.healthcheck {
            let parts: Vec<String> = command.iter().map(|part| q(part)).collect();
            let _ = writeln!(out, "    healthcheck:");
            let _ = writeln!(out, "      command: [{}]", parts.join(", "));
            if let Some(interval) = interval {
                let _ = writeln!(out, "      interval: {interval}");
            }
            if let Some(retries) = retries {
                let _ = writeln!(out, "      retries: {retries}");
            }
        }
        out.push('\n');
    }

    let (secrets, plain): (Vec<&Setting>, Vec<&Setting>) = converter
        .settings
        .values()
        .partition(|setting| setting.secret);
    if !converter.settings.is_empty() {
        let _ = writeln!(out, "settings:");
        if !plain.is_empty() {
            let _ = writeln!(
                out,
                "  general:\n    config:\n      icon: settings\n      fields:"
            );
            for setting in &plain {
                let default = setting
                    .default
                    .as_deref()
                    .map(|value| format!(", default: {}", q(value)))
                    .unwrap_or_default();
                let _ = writeln!(
                    out,
                    "        {}: {{ type: string{default}, restart: true }}",
                    setting.key
                );
            }
        }
        if !secrets.is_empty() {
            let _ = writeln!(
                out,
                "  advanced:\n    secrets:\n      icon: key\n      fields:"
            );
            for setting in &secrets {
                let _ = writeln!(
                    out,
                    "        {}: {{ type: secret, generate: {{ length: 32 }} }}",
                    setting.key
                );
            }
        }
        out.push('\n');
    }

    let _ = writeln!(out, "lifecycle:");
    let _ = writeln!(out, "  service: {}", main.name);
    if main.healthcheck.is_some() {
        let _ = writeln!(out, "  ready: {{ healthcheck: true }}");
    }
    let _ = writeln!(out, "  stop: {{ signal: SIGTERM }}");
    let _ = writeln!(out, "  timeout: 30");
    out
}

/// Parts égales totalisant 100 (le reste au premier service) ; aucune pour un service seul.
fn shares(count: usize) -> Vec<Option<u8>> {
    if count <= 1 {
        return vec![None; count];
    }
    let count_u8 = u8::try_from(count.min(100)).unwrap_or(100);
    let base = 100 / count_u8;
    (0..count)
        .map(|index| {
            Some(if index == 0 {
                base + 100 % count_u8
            } else {
                base
            })
        })
        .collect()
}
