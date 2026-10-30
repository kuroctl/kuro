//! Conversion d'un egg Pterodactyl (`PTDL_v1`, `PTDL_v2`) en brouillon de prefab (ADR-0027).
//!
//! Kuro ne lit pas les eggs : le Studio les traduit une fois en prefab, puis le brouillon suit le
//! chemin habituel (validation, publication). Comme pour l'import compose, ce qui est sûr est
//! converti, le reste est noté par son emplacement dans l'egg.
//!
//! Le démarrage garde le mécanisme des images faites pour Pterodactyl : leur point d'entrée lit
//! `STARTUP` dans l'environnement. Les valeurs des réglages y arrivent par des variables
//! d'environnement, jamais collées dans une ligne de commande.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::LazyLock;

use indexmap::IndexMap;
use regex::Regex;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::compose::{Conversion, Note, NoteKind, q, slug};

/// Taille maximale de l'egg accepté.
pub const MAX_EGG_BYTES: usize = 512 * 1024;

/// Dossier des images Pterodactyl, où le volume du serveur est monté.
const HOME: &str = "/home/container";
const DEFAULT_PORT: u16 = 25565;
const HEREDOC: &str = "KURO_EGG_INSTALL";

static REFERENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\{\{\s*([A-Za-z0-9_.]+)\s*\}\}").unwrap_or_else(|_| unreachable!("static regex"))
});
static PACKAGES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(apt-get|apt|apk|yum|dnf)\s+(-\S+\s+)*(install|add|update|upgrade)\b")
        .unwrap_or_else(|_| unreachable!("static regex"))
});
static SECRET_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(PASSWORD|PASSWD|SECRET|TOKEN|API_?KEY|_KEY$|LICEN[CS]E)")
        .unwrap_or_else(|_| unreachable!("static regex"))
});
/// Variable de port qui n'est pas celui du jeu : console web, RCON, requêtes…
static SIDE_PORT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(TXA|RCON|QUERY|ADMIN|WEB|HTTP|API|PANEL|VOICE|FTP|SFTP|METRICS)")
        .unwrap_or_else(|_| unreachable!("static regex"))
});
/// Paquets demandés par une ligne `apt install …` (et équivalents).
static PACKAGE_NAMES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)\b(?:apt-get|apt|apk|yum|dnf)\s+(?:-\S+\s+)*(?:install|add)\s+([^\n;&|]+)")
        .unwrap_or_else(|_| unreachable!("static regex"))
});
static ENV_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*$").unwrap_or_else(|_| unreachable!("static regex"))
});

#[derive(Debug, thiserror::Error)]
pub enum EggError {
    #[error("egg larger than {MAX_EGG_BYTES} bytes")]
    TooLarge,
    #[error("unreadable egg: {0}")]
    Unreadable(String),
    #[error("not a Pterodactyl egg: {0}")]
    NotAnEgg(&'static str),
}

#[derive(Debug, Deserialize)]
struct Egg {
    #[serde(default)]
    meta: Option<Meta>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    docker_images: IndexMap<String, String>,
    /// `PTDL_v1` : une seule image.
    #[serde(default)]
    image: Option<String>,
    #[serde(default)]
    startup: Option<String>,
    #[serde(default)]
    config: EggConfig,
    #[serde(default)]
    scripts: Option<Scripts>,
    #[serde(default)]
    variables: Vec<Variable>,
    #[serde(default)]
    features: Option<Value>,
    #[serde(default)]
    file_denylist: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct Meta {
    #[serde(default)]
    version: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct EggConfig {
    /// Objet, ou objet encodé en chaîne (export de Pterodactyl).
    #[serde(default)]
    files: Value,
    #[serde(default)]
    startup: Value,
    #[serde(default)]
    stop: Option<String>,
    #[serde(default)]
    logs: Value,
}

#[derive(Debug, Deserialize)]
struct Scripts {
    #[serde(default)]
    installation: Option<Installation>,
}

#[derive(Debug, Deserialize)]
struct Installation {
    #[serde(default)]
    script: Option<String>,
    #[serde(default)]
    container: Option<String>,
    #[serde(default)]
    entrypoint: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Variable {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: Option<String>,
    env_variable: String,
    #[serde(default)]
    default_value: Option<Value>,
    #[serde(default)]
    user_viewable: Value,
    #[serde(default)]
    user_editable: Value,
    #[serde(default)]
    rules: Value,
}

/// Réglage produit à partir d'une variable de l'egg.
#[derive(Debug)]
struct Field {
    key: String,
    env: String,
    label: String,
    help: Option<String>,
    secret: bool,
    /// Ligne YAML du champ, sans la clé (`{ type: int, … }`).
    spec: String,
    /// Valeurs d'une liste, à libeller dans les traductions ; booléen : `0`/`1` ou `false`/`true`.
    options: Vec<String>,
    boolean: bool,
    default: Option<String>,
}

#[derive(Debug)]
struct FileDecl {
    key: String,
    path: String,
    format: Option<&'static str>,
    keys: Vec<(String, String)>,
}

struct Converter {
    notes: Vec<Note>,
}

impl Converter {
    fn note(&mut self, path: impl Into<String>, kind: NoteKind, message: impl Into<String>) {
        self.notes.push(Note {
            path: path.into(),
            kind,
            message: message.into(),
        });
    }
}

/// Valeur JSON vraie à la manière de Pterodactyl (`true`, `1`, `"1"`).
fn truthy(value: &Value) -> bool {
    match value {
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_i64() == Some(1),
        Value::String(text) => matches!(text.as_str(), "1" | "true"),
        _ => false,
    }
}

fn text_of(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

/// `config.files` et consorts : objet, ou objet encodé dans une chaîne.
fn embedded(value: &Value) -> Value {
    match value {
        Value::String(text) if !text.trim().is_empty() => {
            serde_json::from_str(text).unwrap_or(Value::Null)
        }
        Value::String(_) => Value::Null,
        other => other.clone(),
    }
}

/// Chaîne shell entre apostrophes.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// Règles Laravel, en chaîne (`required|max:20`) ou en tableau.
fn rule_list(rules: &Value) -> Vec<String> {
    match rules {
        Value::Array(items) => items.iter().filter_map(text_of).collect(),
        Value::String(text) => {
            let mut out: Vec<String> = Vec::new();
            let mut in_regex = false;
            for token in text.split('|') {
                // Une expression `regex:/a|b/` contient le séparateur : on la recolle.
                if in_regex {
                    if let Some(last) = out.last_mut() {
                        last.push('|');
                        last.push_str(token);
                    }
                } else {
                    out.push(token.trim().to_owned());
                }
                let current = out.last().map(String::as_str).unwrap_or_default();
                in_regex = current.starts_with("regex:") && !regex_closed(current);
            }
            out
        }
        _ => Vec::new(),
    }
}

fn regex_closed(rule: &str) -> bool {
    let body = rule.trim_start_matches("regex:");
    let Some(delimiter) = body.chars().next() else {
        return true;
    };
    body.len() > 1
        && body[1..]
            .trim_end_matches(|c: char| c.is_ascii_alphabetic())
            .ends_with(delimiter)
}

/// `regex:/^[a-z]+$/i` → `(?i)^[a-z]+$`.
fn laravel_pattern(rule: &str) -> Option<String> {
    let body = rule.strip_prefix("regex:")?;
    let delimiter = body.chars().next()?;
    let rest = &body[delimiter.len_utf8()..];
    let end = rest.rfind(delimiter)?;
    let (pattern, flags) = (&rest[..end], &rest[end + delimiter.len_utf8()..]);
    let flags: String = flags.chars().filter(|c| "imsx".contains(*c)).collect();
    Some(if flags.is_empty() {
        pattern.to_owned()
    } else {
        format!("(?{flags}){pattern}")
    })
}

fn number(text: &str) -> Option<String> {
    text.trim().parse::<f64>().ok().map(|n| {
        if n.fract() == 0.0 {
            format!("{}", n as i64)
        } else {
            n.to_string()
        }
    })
}

impl Converter {
    fn field(&mut self, variable: &Variable) -> Option<Field> {
        let at = format!("variables.{}", variable.env_variable);
        if !ENV_NAME.is_match(&variable.env_variable) {
            self.note(at, NoteKind::Refused, "nom de variable invalide");
            return None;
        }
        let key = slug(&variable.env_variable, '_');
        let rules = rule_list(&variable.rules);
        let default = variable
            .default_value
            .as_ref()
            .and_then(text_of)
            .filter(|value| !value.is_empty());
        let required = rules.iter().any(|rule| rule == "required");
        let options: Option<Vec<String>> = rules.iter().find_map(|rule| {
            rule.strip_prefix("in:")
                .map(|list| list.split(',').map(|v| v.trim().to_owned()).collect())
        });
        let boolean = rules.iter().any(|rule| rule == "boolean" || rule == "bool");
        let integer = rules.iter().any(|rule| rule == "integer" || rule == "int");
        let numeric = rules.iter().any(|rule| rule == "numeric");
        let secret = SECRET_NAME.is_match(&variable.env_variable);

        let mut parts: Vec<String> = Vec::new();
        let mut listed: Vec<String> = Vec::new();
        let mut default_out = default.clone();
        let kind = if let Some(options) = &options {
            let list: Vec<String> = options.iter().map(|o| q(o)).collect();
            parts.push(format!("options: [{}]", list.join(", ")));
            listed.clone_from(options);
            if let Some(value) = &default
                && !options.contains(value)
            {
                self.note(
                    format!("{at}.default_value"),
                    NoteKind::Adapted,
                    "valeur par défaut absente de la liste : première option retenue",
                );
                default_out = options.first().cloned();
            }
            "enum"
        } else if boolean {
            // Les valeurs restent celles que l'image attend (`0`/`1` ou `true`/`false`).
            let numeric_flags = default.as_deref().is_none_or(|v| v == "0" || v == "1");
            let flags = if numeric_flags {
                ["0", "1"]
            } else {
                ["false", "true"]
            };
            parts.push(format!("options: [{}, {}]", q(flags[0]), q(flags[1])));
            listed = flags.iter().map(|flag| (*flag).to_owned()).collect();
            if default_out
                .as_deref()
                .is_some_and(|value| !flags.contains(&value))
            {
                default_out = Some(flags[0].to_owned());
            }
            self.note(
                &at,
                NoteKind::Adapted,
                format!("booléen converti en liste ({} / {})", flags[0], flags[1]),
            );
            "enum"
        } else if integer || numeric {
            if let Some(value) = &default {
                match number(value) {
                    Some(parsed) => default_out = Some(parsed),
                    None => {
                        self.note(
                            format!("{at}.default_value"),
                            NoteKind::Ignored,
                            "valeur par défaut non numérique",
                        );
                        default_out = None;
                    }
                }
            }
            if integer { "int" } else { "float" }
        } else if secret {
            "password"
        } else {
            "string"
        };
        let numeric_kind = matches!(kind, "int" | "float");

        for rule in &rules {
            let (name, argument) = rule.split_once(':').unwrap_or((rule.as_str(), ""));
            match name {
                "required" | "nullable" | "sometimes" | "string" | "integer" | "int"
                | "numeric" | "boolean" | "bool" | "in" | "" => {}
                "max" if numeric_kind => parts.push(format!("max: {argument}")),
                "min" if numeric_kind => parts.push(format!("min: {argument}")),
                "between" if numeric_kind => {
                    if let Some((low, high)) = argument.split_once(',') {
                        parts.push(format!("min: {}, max: {}", low.trim(), high.trim()));
                    }
                }
                "max" | "size" if kind != "enum" => {
                    if argument.trim().parse::<usize>().is_ok() {
                        parts.push(format!("max_length: {}", argument.trim()));
                    }
                }
                "between" if kind != "enum" => {
                    if let Some((_, high)) = argument.split_once(',') {
                        parts.push(format!("max_length: {}", high.trim()));
                    }
                }
                "regex" if kind != "enum" => match laravel_pattern(rule) {
                    Some(pattern) => parts.push(format!("pattern: {}", q(&pattern))),
                    None => self.note(
                        format!("{at}.rules"),
                        NoteKind::Ignored,
                        "expression régulière illisible",
                    ),
                },
                "min" => {}
                other => self.note(
                    format!("{at}.rules"),
                    NoteKind::Ignored,
                    format!("règle `{other}` sans équivalent"),
                ),
            }
        }

        let mut spec = format!("type: {kind}");
        if let Some(value) = &default_out {
            if numeric_kind {
                let _ = write!(spec, ", default: {value}");
            } else {
                let _ = write!(spec, ", default: {}", q(value));
            }
        }
        if required && default_out.is_none() {
            spec.push_str(", required: true");
        }
        for part in parts {
            let _ = write!(spec, ", {part}");
        }
        if truthy(&variable.user_viewable) && truthy(&variable.user_editable) {
            spec.push_str(", visibility: customer");
        } else if truthy(&variable.user_viewable) {
            self.note(
                &at,
                NoteKind::Adapted,
                "visible mais non modifiable par l'utilisateur : réservé à l'admin",
            );
        }
        spec.push_str(", restart: true");
        Some(Field {
            key,
            env: variable.env_variable.clone(),
            label: if variable.name.is_empty() {
                variable.env_variable.clone()
            } else {
                variable.name.clone()
            },
            help: variable.description.clone().filter(|d| !d.is_empty()),
            secret,
            spec: format!("{{ {spec} }}"),
            options: listed,
            boolean: boolean && options.is_none(),
            default: default_out,
        })
    }

    /// Référence Pterodactyl (`server.build.default.port`, `server.build.env.X`) → gabarit Kuro.
    fn reference(&self, reference: &str, fields: &[Field]) -> Option<String> {
        let setting = |env: &str| {
            fields
                .iter()
                .find(|field| field.env == env)
                .map(|field| format!("{{{{ settings.{} }}}}", field.key))
                .or_else(|| wings(env))
        };
        match reference {
            "server.build.default.port" => Some("{{ ports.game.container }}".to_owned()),
            "server.build.default.ip" => Some("0.0.0.0".to_owned()),
            "server.build.memory" => Some("{{ resources.memory_mb }}".to_owned()),
            "server.uuid" => Some("{{ server.id }}".to_owned()),
            other => other
                .strip_prefix("server.build.env.")
                .or_else(|| other.strip_prefix("env."))
                .and_then(setting),
        }
    }

    fn files(&mut self, value: &Value, fields: &[Field]) -> Vec<FileDecl> {
        let Value::Object(map) = embedded(value) else {
            return Vec::new();
        };
        let mut out: Vec<FileDecl> = Vec::new();
        for (name, spec) in &map {
            let at = format!("config.files.{name}");
            let relative = name.trim_start_matches("./").trim_start_matches('/');
            if relative.is_empty() || relative.split('/').any(|part| part == "..") {
                self.note(at, NoteKind::Refused, "chemin hors du dossier du serveur");
                continue;
            }
            let parser = spec.get("parser").and_then(Value::as_str).unwrap_or("file");
            let format = match parser {
                "properties" => Some("properties"),
                "ini" => Some("ini"),
                _ => None,
            };
            let mut keys = Vec::new();
            if let Some(find) = spec.get("find").and_then(Value::as_object) {
                if format.is_none() {
                    self.note(
                        format!("{at}.find"),
                        NoteKind::Refused,
                        format!(
                            "Kuro ne réécrit pas de clé dans le format `{parser}` : fichier déclaré éditable à la main"
                        ),
                    );
                } else {
                    for (key, raw) in find {
                        let Some(text) = text_of(raw) else {
                            self.note(
                                format!("{at}.find.{key}"),
                                NoteKind::Refused,
                                "remplacement conditionnel sans équivalent",
                            );
                            continue;
                        };
                        match self.template(&text, fields) {
                            Ok(converted) => keys.push((key.clone(), converted)),
                            Err(reference) => self.note(
                                format!("{at}.find.{key}"),
                                NoteKind::Refused,
                                format!("référence `{reference}` sans équivalent : clé retirée"),
                            ),
                        }
                    }
                }
            }
            let mut file_key = slug(relative, '-');
            let base = file_key.clone();
            let mut index = 2;
            while out.iter().any(|file| file.key == file_key) {
                file_key = format!("{base}-{index}");
                index += 1;
            }
            out.push(FileDecl {
                key: file_key,
                path: format!("{HOME}/{relative}"),
                format,
                keys,
            });
        }
        out
    }

    /// Texte avec références Pterodactyl → gabarit Kuro ; `Err(référence)` si l'une est inconnue.
    fn template(&self, text: &str, fields: &[Field]) -> Result<String, String> {
        // Accolades restées dans le texte de l'egg : le moteur de gabarits de Kuro les lirait.
        if REFERENCE.replace_all(text, "").contains("{{") {
            return Err("{{".to_owned());
        }
        let mut out = String::new();
        let mut last = 0;
        for capture in REFERENCE.captures_iter(text) {
            let whole = capture.get(0).map_or(0..0, |m| m.range());
            out.push_str(&text[last..whole.start]);
            let reference = &capture[1];
            out.push_str(
                &self
                    .reference(reference, fields)
                    .ok_or(reference.to_owned())?,
            );
            last = whole.end;
        }
        out.push_str(&text[last..]);
        Ok(out)
    }

    /// `startup` → valeur de `STARTUP` : `{{VAR}}` devient `${VAR}`, lu par le point d'entrée.
    fn startup(&mut self, startup: &str) -> String {
        let converted = REFERENCE.replace_all(startup, |capture: &regex::Captures<'_>| {
            let reference = &capture[1];
            let name = match reference {
                "server.build.default.port" => "SERVER_PORT",
                "server.build.default.ip" => "SERVER_IP",
                "server.build.memory" => "SERVER_MEMORY",
                other => other
                    .strip_prefix("server.build.env.")
                    .or_else(|| other.strip_prefix("env."))
                    .unwrap_or(other),
            };
            format!("${{{name}}}")
        });
        let converted = converted.into_owned();
        if converted.contains("{{") || converted.contains("}}") {
            self.note(
                "startup",
                NoteKind::Refused,
                "accolades doubles restantes : retirées",
            );
            return converted.replace("{{", "").replace("}}", "");
        }
        converted
    }
}

/// Variables d'environnement fournies par Wings, reproduites par Kuro.
fn wings(env: &str) -> Option<String> {
    match env {
        "SERVER_PORT" => Some("{{ ports.game.container }}".to_owned()),
        "SERVER_MEMORY" => Some("{{ resources.memory_mb }}".to_owned()),
        "SERVER_IP" => Some("0.0.0.0".to_owned()),
        "P_SERVER_UUID" => Some("{{ server.id }}".to_owned()),
        _ => None,
    }
}

/// Ports de l'egg : celui du jeu, et ceux de ses variables `*PORT*` qui servent à autre chose.
struct Ports {
    game: u16,
    /// Le port du jeu vient de l'egg (sinon : valeur par défaut, à vérifier).
    known: bool,
    /// (nom du port, variable, numéro) : console web, RCON, requêtes…
    side: Vec<(String, String, u16)>,
}

/// Une variable `*PORT*` numérique de l'egg donne le port du jeu, sauf si son nom désigne un
/// autre service (`TXHOST_TXA_PORT`, `RCON_PORT`…) : elle devient alors un port à part.
fn guess_ports(egg: &Egg) -> Ports {
    let mut game = None;
    let mut side = Vec::new();
    for variable in &egg.variables {
        let name = variable.env_variable.to_uppercase();
        if !name.contains("PORT") {
            continue;
        }
        let Some(port) = variable
            .default_value
            .as_ref()
            .and_then(text_of)
            .and_then(|value| value.trim().parse::<u16>().ok())
            .filter(|port| *port >= 1024)
        else {
            continue;
        };
        if SIDE_PORT.is_match(&name) {
            side.push((
                slug(&variable.env_variable, '_'),
                variable.env_variable.clone(),
                port,
            ));
        } else if game.is_none() {
            game = Some(port);
        }
    }
    match game {
        Some(port) => Ports {
            game: port,
            known: true,
            side,
        },
        None => Ports {
            game: DEFAULT_PORT,
            known: egg.name.to_lowercase().contains("minecraft"),
            side,
        },
    }
}

/// Texte de `lifecycle.ready` : textes de l'egg échappés, réunis par `|`.
fn ready_pattern(startup: &Value) -> Option<String> {
    let done = embedded(startup).get("done").cloned()?;
    let texts: Vec<String> = match done {
        Value::String(text) => vec![text],
        Value::Array(items) => items.iter().filter_map(text_of).collect(),
        _ => Vec::new(),
    };
    let texts: Vec<String> = texts
        .iter()
        .filter(|text| !text.is_empty())
        .map(|text| regex::escape(text))
        .collect();
    match texts.len() {
        0 => None,
        1 => texts.into_iter().next(),
        _ => Some(format!("({})", texts.join("|"))),
    }
}

/// Convertit un egg. `id` : identifiant voulu (sinon dérivé du nom de l'egg).
pub fn convert(text: &str, id: Option<&str>) -> Result<Conversion, EggError> {
    if text.len() > MAX_EGG_BYTES {
        return Err(EggError::TooLarge);
    }
    let egg: Egg =
        serde_json::from_str(text).map_err(|error| EggError::Unreadable(error.to_string()))?;
    let version = egg.meta.as_ref().and_then(|meta| meta.version.as_deref());
    if !matches!(version, Some("PTDL_v1" | "PTDL_v2")) {
        return Err(EggError::NotAnEgg("meta.version is not PTDL_v1 or PTDL_v2"));
    }
    let mut images: Vec<(String, String)> = egg
        .docker_images
        .iter()
        .map(|(label, image)| (label.clone(), image.clone()))
        .collect();
    if let Some(image) = &egg.image {
        images.push((String::new(), image.clone()));
    }
    let Some((_, image)) = images.first().cloned() else {
        return Err(EggError::NotAnEgg("no docker image"));
    };
    let startup = egg
        .startup
        .clone()
        .filter(|s| !s.trim().is_empty())
        .ok_or(EggError::NotAnEgg("no startup command"))?;

    let mut converter = Converter { notes: Vec::new() };
    converter.note(
        "docker_images",
        NoteKind::Adapted,
        format!(
            "image `{image}` : tag à épingler par empreinte (@sha256) ou tag daté avant publication"
        ),
    );
    for (label, other) in images.iter().skip(1) {
        converter.note(
            format!("docker_images.{label}"),
            NoteKind::Ignored,
            format!(
                "`{other}` : un prefab a une seule image ; publier une version par image si besoin"
            ),
        );
    }
    let author_note = "non repris : adresse e-mail";
    converter.note("author", NoteKind::Ignored, author_note);
    if egg.features.as_ref().is_some_and(|f| !f.is_null()) {
        converter.note(
            "features",
            NoteKind::Ignored,
            "fonctions propres au panel Pterodactyl (EULA, version de Java…)",
        );
    }
    if egg
        .file_denylist
        .as_ref()
        .is_some_and(|f| f.as_array().is_some_and(|a| !a.is_empty()))
    {
        converter.note(
            "file_denylist",
            NoteKind::Ignored,
            "les fichiers protégés se déclarent dans `files`",
        );
    }
    if !embedded(&egg.config.logs).is_null() {
        converter.note(
            "config.logs",
            NoteKind::Ignored,
            "la console est relayée par Kuro",
        );
    }

    let mut fields: Vec<Field> = Vec::new();
    for variable in &egg.variables {
        if let Some(field) = converter.field(variable)
            && !fields.iter().any(|f| f.key == field.key)
        {
            fields.push(field);
        }
    }
    let startup_value = converter.startup(&startup);
    let files = converter.files(&egg.config.files, &fields);
    let ready = ready_pattern(&egg.config.startup);
    if ready.is_none() {
        converter.note(
            "config.startup.done",
            NoteKind::Adapted,
            "aucun texte « serveur prêt » : le serveur est considéré prêt dès son démarrage",
        );
    }
    let stop = match egg.config.stop.as_deref().map(str::trim) {
        Some("^C" | "^^C") => "{ signal: SIGINT }".to_owned(),
        Some(command) if !command.is_empty() && !command.starts_with('^') => {
            format!("{{ console: {} }}", q(command))
        }
        Some(other) if !other.is_empty() => {
            converter.note(
                "config.stop",
                NoteKind::Adapted,
                format!("signal `{other}` remplacé par SIGTERM"),
            );
            "{ signal: SIGTERM }".to_owned()
        }
        _ => "{ signal: SIGTERM }".to_owned(),
    };
    let ports = guess_ports(&egg);
    let (port, known) = (ports.game, ports.known);
    for (name, variable, number) in &ports.side {
        converter.note(
            format!("variables.{variable}"),
            NoteKind::Adapted,
            format!(
                "port `{name}` ({number}) publié à part, en TCP : protocole et visibilité à vérifier"
            ),
        );
    }
    if !known {
        converter.note(
            "startup",
            NoteKind::Adapted,
            format!(
                "port du jeu inconnu de l'egg : {port} par défaut, à vérifier (et à compléter si le jeu ouvre d'autres ports)"
            ),
        );
    }
    let install = egg
        .scripts
        .as_ref()
        .and_then(|scripts| scripts.installation.as_ref())
        .filter(|installation| {
            installation
                .script
                .as_deref()
                .is_some_and(|script| !script.trim().is_empty())
        })
        .map(|installation| converter.install(installation, &fields));
    converter.note(
        "variables",
        NoteKind::Adapted,
        "traduction française à compléter (textes de l'egg repris en anglais)",
    );

    let id = slug(id.unwrap_or(&egg.name), '-');
    let mut id: String = id.chars().take(48).collect();
    id = id.trim_end_matches('-').to_owned();
    if id.is_empty() {
        id = "imported-egg".to_owned();
    }

    let manifest = write_manifest(&ManifestParts {
        id: &id,
        image: &image,
        startup: &startup_value,
        port,
        side_ports: &ports.side,
        fields: &fields,
        files: &files,
        install: install.as_ref(),
        ready: ready.as_deref(),
        stop: &stop,
        notes: &converter.notes,
    });
    let title = if egg.name.trim().is_empty() {
        id.clone()
    } else {
        egg.name.trim().to_owned()
    };
    let description = egg.description.clone().unwrap_or_default();
    let field_texts = |lang: &str| -> serde_json::Map<String, Value> {
        let (no, yes) = if lang == "fr" {
            ("Non", "Oui")
        } else {
            ("No", "Yes")
        };
        fields
            .iter()
            .map(|field| {
                let mut text = json!({ "label": field.label });
                if let Some(help) = &field.help {
                    text["help"] = json!(help);
                }
                if !field.options.is_empty() {
                    let labels: serde_json::Map<String, Value> = field
                        .options
                        .iter()
                        .enumerate()
                        .map(|(index, option)| {
                            let label = match (field.boolean, index) {
                                (true, 0) => no.to_owned(),
                                (true, _) => yes.to_owned(),
                                _ => option.clone(),
                            };
                            (option.clone(), json!(label))
                        })
                        .collect();
                    text["options"] = Value::Object(labels);
                }
                (field.key.clone(), text)
            })
            .collect()
    };
    let file_texts: serde_json::Map<String, Value> = files
        .iter()
        .map(|file| (file.key.clone(), json!({ "description": file.path })))
        .collect();
    let locale = |lang: &str| {
        let (config, security) = if lang == "fr" {
            ("Configuration", "Sécurité")
        } else {
            ("Configuration", "Security")
        };
        format!(
            "{}\n",
            serde_json::to_string_pretty(&json!({
                "prefab": { "name": title, "description": description },
                "groups": { "config": { "title": config }, "security": { "title": security } },
                "fields": field_texts(lang),
                "files": file_texts,
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

/// Installation convertie : image et script prêt pour `sh -c`.
#[derive(Debug)]
struct Install {
    image: Option<String>,
    run: String,
}

impl Converter {
    fn install(&mut self, installation: &Installation, fields: &[Field]) -> Install {
        let at = "scripts.installation";
        let script = installation
            .script
            .as_deref()
            .unwrap_or_default()
            .replace("\r\n", "\n")
            .replace("/mnt/server", HOME);
        if PACKAGES.is_match(&script) {
            let mut packages: Vec<&str> = PACKAGE_NAMES
                .captures_iter(&script)
                .flat_map(|capture| capture.get(1).map_or("", |m| m.as_str()).split_whitespace())
                .filter(|word| !word.starts_with('-'))
                .collect();
            packages.dedup();
            let wanted = if packages.is_empty() {
                String::new()
            } else {
                format!(" ({})", packages.join(", "))
            };
            self.note(
                format!("{at}.script"),
                NoteKind::Refused,
                format!(
                    "installe des paquets système{wanted} : impossible sans root. Le script reste dans le brouillon, à réécrire (étapes download/extract/steamcmd, ou image d'installation qui contient déjà ces outils)"
                ),
            );
        }
        self.note(
            format!("{at}.script"),
            NoteKind::Adapted,
            format!(
                "lancé sans root, `/mnt/server` remplacé par `{HOME}` ; les variables y prennent leur valeur par défaut (le conteneur d'installation ne reçoit pas les réglages)"
            ),
        );
        let mut defaults = String::from("# Valeurs par défaut des variables de l'egg.\n");
        for field in fields {
            let value = field.default.as_deref().unwrap_or_default();
            let _ = writeln!(defaults, "export {}={}", field.env, shell_quote(value));
        }
        let body = format!("{defaults}{}", script.trim_end());
        let bash = installation
            .entrypoint
            .as_deref()
            .is_some_and(|entrypoint| entrypoint.contains("bash"));
        let run = if bash && !body.lines().any(|line| line.trim() == HEREDOC) {
            format!("exec bash <<'{HEREDOC}'\n{body}\n{HEREDOC}\n")
        } else {
            format!("{body}\n")
        };
        if let Some(image) = &installation.container {
            self.note(
                format!("{at}.container"),
                NoteKind::Adapted,
                format!("image d'installation `{image}` : à épingler, et lancée avec l'utilisateur du serveur"),
            );
        }
        Install {
            image: installation.container.clone().filter(|i| !i.is_empty()),
            run,
        }
    }
}

struct ManifestParts<'a> {
    id: &'a str,
    image: &'a str,
    startup: &'a str,
    port: u16,
    side_ports: &'a [(String, String, u16)],
    fields: &'a [Field],
    files: &'a [FileDecl],
    install: Option<&'a Install>,
    ready: Option<&'a str>,
    stop: &'a str,
    notes: &'a [Note],
}

fn write_manifest(parts: &ManifestParts<'_>) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# Converti depuis un egg Pterodactyl (Prefab Studio, ADR-0027). À relire avant publication :"
    );
    let _ = writeln!(
        out,
        "# image épinglée, port du jeu, script d'installation (lancé sans root)."
    );
    for note in parts.notes.iter().filter(|n| n.kind == NoteKind::Refused) {
        let _ = writeln!(out, "# refusé : {} : {}", note.path, note.message);
    }
    let _ = writeln!(out, "schema: kuro.prefab/v1");
    let _ = writeln!(out, "id: {}", parts.id);
    let _ = writeln!(out, "version: 0.1.0");
    let _ = writeln!(out, "category: game");
    let _ = writeln!(out, "tags: []");
    let _ = writeln!(out, "requires:\n  kuro: \">=0.2\"\n");

    let _ = writeln!(out, "services:");
    let _ = writeln!(out, "  server:");
    let _ = writeln!(out, "    image: {}", q(parts.image));
    let _ = writeln!(out, "    env:");
    let _ = writeln!(
        out,
        "      # Lu par le point d'entrée de l'image, qui remplace ${{VAR}} par l'environnement."
    );
    let _ = writeln!(out, "      STARTUP: {}", q(parts.startup));
    let mut declared: Vec<&str> = Vec::new();
    for field in parts.fields {
        let _ = writeln!(
            out,
            "      {}: \"{{{{ settings.{} }}}}\"",
            field.env, field.key
        );
        declared.push(&field.env);
    }
    for env in ["SERVER_PORT", "SERVER_MEMORY", "SERVER_IP", "P_SERVER_UUID"] {
        if !declared.contains(&env)
            && let Some(value) = wings(env)
        {
            let _ = writeln!(out, "      {env}: {}", q(&value));
        }
    }
    let _ = writeln!(out, "    ports:");
    let _ = writeln!(
        out,
        "      game: {{ container: {}, protocol: tcp, public: true }}",
        parts.port
    );
    for (name, _, number) in parts.side_ports {
        if name != "game" {
            let _ = writeln!(
                out,
                "      {name}: {{ container: {number}, protocol: tcp, public: true }}"
            );
        }
    }
    let _ = writeln!(out, "    volumes:");
    let _ = writeln!(out, "      data: {{ path: {HOME} }}");
    let _ = writeln!(out, "    writable: [/tmp]");
    let _ = writeln!(
        out,
        "    # Utilisateur des images `yolks` de Pterodactyl ; à vérifier pour une autre image."
    );
    let _ = writeln!(out, "    user: \"1000:1000\"\n");

    if !parts.fields.is_empty() {
        let (secrets, plain): (Vec<&Field>, Vec<&Field>) =
            parts.fields.iter().partition(|field| field.secret);
        let _ = writeln!(out, "settings:");
        if !plain.is_empty() {
            let _ = writeln!(
                out,
                "  general:\n    config:\n      icon: settings\n      fields:"
            );
            for field in plain {
                let _ = writeln!(out, "        {}: {}", field.key, field.spec);
            }
        }
        if !secrets.is_empty() {
            let _ = writeln!(
                out,
                "  advanced:\n    security:\n      icon: shield\n      fields:"
            );
            for field in secrets {
                let _ = writeln!(out, "        {}: {}", field.key, field.spec);
            }
        }
        out.push('\n');
    }

    if !parts.files.is_empty() {
        let _ = writeln!(out, "files:");
        for file in parts.files {
            let _ = writeln!(out, "  {}:", file.key);
            let _ = writeln!(out, "    service: server");
            let _ = writeln!(out, "    path: {}", q(&file.path));
            match file.format {
                Some(format) if !file.keys.is_empty() => {
                    let _ = writeln!(out, "    format: {format}");
                    let _ = writeln!(out, "    mode: managed-keys");
                    let _ = writeln!(out, "    keys:");
                    for (key, value) in &file.keys {
                        let _ = writeln!(out, "      {}: {}", q(key), q(value));
                    }
                }
                format => {
                    let _ = writeln!(out, "    format: {}", format.unwrap_or("text"));
                    let _ = writeln!(out, "    mode: user");
                }
            }
        }
        out.push('\n');
    }

    if let Some(install) = parts.install {
        let _ = writeln!(out, "install:");
        let _ = writeln!(out, "  service: server");
        let _ = writeln!(out, "  steps:");
        let _ = writeln!(out, "    - script:");
        if let Some(image) = &install.image {
            let _ = writeln!(out, "        image: {}", q(image));
        }
        let _ = writeln!(out, "        run: |");
        for line in install.run.lines() {
            if line.trim().is_empty() {
                out.push('\n');
            } else {
                let _ = writeln!(out, "          {line}");
            }
        }
        out.push('\n');
    }

    let _ = writeln!(out, "lifecycle:");
    let _ = writeln!(out, "  service: server");
    if let Some(ready) = parts.ready {
        let _ = writeln!(out, "  ready: {{ console: {} }}", q(ready));
    }
    let _ = writeln!(out, "  stop: {}", parts.stop);
    let _ = writeln!(out, "  timeout: 60");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn laravel_rules_keep_a_regex_with_a_pipe() {
        let rules = rule_list(&json!("required|regex:/^(a|b)$/|max:20"));
        assert_eq!(rules, ["required", "regex:/^(a|b)$/", "max:20"]);
        assert_eq!(
            laravel_pattern("regex:/^[a-z]+$/i").as_deref(),
            Some("(?i)^[a-z]+$")
        );
    }

    #[test]
    fn shell_values_are_quoted() {
        assert_eq!(shell_quote("it's $HOME"), r"'it'\''s $HOME'");
    }
}
