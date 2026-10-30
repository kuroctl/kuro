//! Gabarits `{{ référence | filtre }}`, sans logique (ADR-0011).
//!
//! Références : `settings.<champ>`, `ports.<nom>.host` / `.container`, `resources.memory_mb` /
//! `.cpu_percent`, `server.id` / `.name`, `content.<emplacement>.ids`,
//! `content.<emplacement>.values.<champ>` (ADR-0030).
//! Filtres : `join("sep")`, `prefix("…")` (chaque élément d'une liste), `lower`, `upper`,
//! `default("…")`. Rien d'autre : un prefab ne peut
//! ni boucler, ni appeler de code, et le rendu est déterministe.

use std::collections::BTreeMap;
use std::fmt;

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TemplateError {
    #[error("unclosed `{{{{`")]
    Unclosed,
    #[error("invalid reference `{0}`")]
    InvalidReference(String),
    #[error("unknown filter `{0}`")]
    UnknownFilter(String),
    #[error("filter `{0}`: invalid argument")]
    InvalidArgument(String),
    #[error("`{0}` has no value")]
    Missing(String),
    #[error("`{0}` is a list: use `join`")]
    ListWithoutJoin(String),
}

/// Ce qu'une référence désigne.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reference {
    Setting(String),
    PortHost(String),
    PortContainer(String),
    MemoryMb,
    CpuPercent,
    ServerId,
    ServerName,
    ContentIds(String),
    /// Valeurs d'un champ de l'emplacement, pour tous ses contenus actifs.
    ContentValues(String, String),
}

impl Reference {
    fn parse(text: &str) -> Result<Self, TemplateError> {
        let parts: Vec<&str> = text.split('.').collect();
        let invalid = || TemplateError::InvalidReference(text.to_owned());
        let name = |part: &str| {
            if is_key(part) || is_slug(part) {
                Ok(part.to_owned())
            } else {
                Err(invalid())
            }
        };
        match parts.as_slice() {
            ["settings", field] => Ok(Self::Setting(name(field)?)),
            ["ports", port, "host"] => Ok(Self::PortHost(name(port)?)),
            ["ports", port, "container"] => Ok(Self::PortContainer(name(port)?)),
            ["resources", "memory_mb"] => Ok(Self::MemoryMb),
            ["resources", "cpu_percent"] => Ok(Self::CpuPercent),
            ["server", "id"] => Ok(Self::ServerId),
            ["server", "name"] => Ok(Self::ServerName),
            ["content", slot, "ids"] => Ok(Self::ContentIds(name(slot)?)),
            ["content", slot, "values", field] => {
                Ok(Self::ContentValues(name(slot)?, name(field)?))
            }
            _ => Err(invalid()),
        }
    }
}

impl fmt::Display for Reference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Setting(key) => write!(f, "settings.{key}"),
            Self::PortHost(port) => write!(f, "ports.{port}.host"),
            Self::PortContainer(port) => write!(f, "ports.{port}.container"),
            Self::MemoryMb => f.write_str("resources.memory_mb"),
            Self::CpuPercent => f.write_str("resources.cpu_percent"),
            Self::ServerId => f.write_str("server.id"),
            Self::ServerName => f.write_str("server.name"),
            Self::ContentIds(slot) => write!(f, "content.{slot}.ids"),
            Self::ContentValues(slot, field) => write!(f, "content.{slot}.values.{field}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Filter {
    Join(String),
    /// Préfixe chaque élément d'une liste, ou un texte non vide.
    Prefix(String),
    Lower,
    Upper,
    Default(String),
}

impl Filter {
    fn parse(text: &str) -> Result<Self, TemplateError> {
        let (name, argument) = match text.split_once('(') {
            Some((name, rest)) => {
                let argument = rest
                    .strip_suffix(')')
                    .map(str::trim)
                    .and_then(|arg| arg.strip_prefix('"')?.strip_suffix('"'))
                    .ok_or_else(|| TemplateError::InvalidArgument(name.trim().to_owned()))?;
                (name.trim(), Some(argument.to_owned()))
            }
            None => (text.trim(), None),
        };
        match (name, argument) {
            ("join", Some(sep)) => Ok(Self::Join(sep)),
            ("prefix", Some(prefix)) => Ok(Self::Prefix(prefix)),
            ("default", Some(value)) => Ok(Self::Default(value)),
            ("lower", None) => Ok(Self::Lower),
            ("upper", None) => Ok(Self::Upper),
            ("join" | "prefix" | "default" | "lower" | "upper", _) => {
                Err(TemplateError::InvalidArgument(name.to_owned()))
            }
            _ => Err(TemplateError::UnknownFilter(name.to_owned())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Text(String),
    Expr {
        reference: Reference,
        filters: Vec<Filter>,
    },
}

/// Gabarit analysé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    parts: Vec<Part>,
}

impl Template {
    pub fn parse(source: &str) -> Result<Self, TemplateError> {
        let mut parts = Vec::new();
        let mut rest = source;
        while let Some(start) = rest.find("{{") {
            if start > 0 {
                parts.push(Part::Text(rest[..start].to_owned()));
            }
            let after = &rest[start + 2..];
            let end = after.find("}}").ok_or(TemplateError::Unclosed)?;
            let mut pieces = split_pipes(&after[..end]).into_iter();
            let reference = Reference::parse(pieces.next().unwrap_or_default().trim())?;
            let filters = pieces
                .map(|piece| Filter::parse(piece.trim()))
                .collect::<Result<_, _>>()?;
            parts.push(Part::Expr { reference, filters });
            rest = &after[end + 2..];
        }
        if !rest.is_empty() {
            parts.push(Part::Text(rest.to_owned()));
        }
        Ok(Self { parts })
    }

    /// Références utilisées, pour la validation du prefab.
    pub fn references(&self) -> impl Iterator<Item = &Reference> {
        self.parts.iter().filter_map(|part| match part {
            Part::Expr { reference, .. } => Some(reference),
            Part::Text(_) => None,
        })
    }

    /// Vrai si l'expression de cette référence se termine par `join`.
    pub fn joins(&self, target: &Reference) -> bool {
        self.parts.iter().any(|part| match part {
            Part::Expr { reference, filters } => {
                reference == target && filters.iter().any(|f| matches!(f, Filter::Join(_)))
            }
            Part::Text(_) => false,
        })
    }

    pub fn render(&self, context: &Context<'_>) -> Result<String, TemplateError> {
        let mut out = String::new();
        for part in &self.parts {
            match part {
                Part::Text(text) => out.push_str(text),
                Part::Expr { reference, filters } => {
                    out.push_str(&render_expr(reference, filters, context)?);
                }
            }
        }
        Ok(out)
    }
}

/// Découpe sur `|` hors des chaînes entre guillemets (`join("|")`).
fn split_pipes(expr: &str) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    for (index, ch) in expr.char_indices() {
        match ch {
            '"' => quoted = !quoted,
            '|' if !quoted => {
                pieces.push(&expr[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    pieces.push(&expr[start..]);
    pieces
}

/// Valeurs disponibles au rendu.
#[derive(Debug, Clone, Default)]
pub struct Context<'a> {
    pub settings: Option<&'a BTreeMap<String, Value>>,
    /// Port → (port de l'hôte s'il est publié, port du conteneur).
    pub ports: BTreeMap<String, (Option<u16>, u16)>,
    pub memory_mb: u32,
    pub cpu_percent: u32,
    pub server_id: String,
    pub server_name: String,
    pub content: BTreeMap<String, Vec<String>>,
    /// `<emplacement>.<champ>` → valeurs des contenus actifs, dans l'ordre d'ajout.
    pub content_values: BTreeMap<String, Vec<String>>,
}

enum Rendered {
    Text(String),
    List(Vec<String>),
}

fn render_expr(
    reference: &Reference,
    filters: &[Filter],
    context: &Context<'_>,
) -> Result<String, TemplateError> {
    let missing = || TemplateError::Missing(reference.to_string());
    let mut value = match reference {
        // Réglage facultatif sans valeur : chaîne vide (les champs obligatoires sont vérifiés
        // à la saisie). `default("…")` permet d'imposer une autre valeur.
        Reference::Setting(key) => Some(Rendered::Text(
            context
                .settings
                .and_then(|s| s.get(key))
                .map(scalar)
                .unwrap_or_default(),
        )),
        Reference::PortHost(port) => context
            .ports
            .get(port)
            .and_then(|(host, _)| *host)
            .map(|p| Rendered::Text(p.to_string())),
        Reference::PortContainer(port) => context
            .ports
            .get(port)
            .map(|(_, container)| Rendered::Text(container.to_string())),
        Reference::MemoryMb => Some(Rendered::Text(context.memory_mb.to_string())),
        Reference::CpuPercent => Some(Rendered::Text(context.cpu_percent.to_string())),
        Reference::ServerId => Some(Rendered::Text(context.server_id.clone())),
        Reference::ServerName => Some(Rendered::Text(context.server_name.clone())),
        Reference::ContentIds(slot) => Some(Rendered::List(
            context.content.get(slot).cloned().unwrap_or_default(),
        )),
        Reference::ContentValues(slot, field) => Some(Rendered::List(
            context
                .content_values
                .get(&format!("{slot}.{field}"))
                .cloned()
                .unwrap_or_default(),
        )),
    };

    for filter in filters {
        value = match (filter, value) {
            (Filter::Default(fallback), None) => Some(Rendered::Text(fallback.clone())),
            (Filter::Default(fallback), Some(Rendered::Text(text))) if text.is_empty() => {
                Some(Rendered::Text(fallback.clone()))
            }
            (Filter::Join(sep), Some(Rendered::List(items))) => {
                Some(Rendered::Text(items.join(sep)))
            }
            (Filter::Prefix(prefix), Some(Rendered::List(items))) => Some(Rendered::List(
                items
                    .into_iter()
                    .map(|item| format!("{prefix}{item}"))
                    .collect(),
            )),
            (Filter::Prefix(prefix), Some(Rendered::Text(text))) if !text.is_empty() => {
                Some(Rendered::Text(format!("{prefix}{text}")))
            }
            (Filter::Lower, Some(Rendered::Text(text))) => {
                Some(Rendered::Text(text.to_lowercase()))
            }
            (Filter::Upper, Some(Rendered::Text(text))) => {
                Some(Rendered::Text(text.to_uppercase()))
            }
            (_, other) => other,
        };
    }
    match value {
        Some(Rendered::Text(text)) => Ok(text),
        Some(Rendered::List(_)) => Err(TemplateError::ListWithoutJoin(reference.to_string())),
        None => Err(missing()),
    }
}

/// Valeur d'un réglage telle qu'insérée dans un gabarit.
pub fn scalar(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        other => other.to_string(),
    }
}

pub(crate) fn is_key(text: &str) -> bool {
    let mut chars = text.chars();
    matches!(chars.next(), Some('a'..='z'))
        && text.len() <= 63
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

pub(crate) fn is_slug(text: &str) -> bool {
    let mut chars = text.chars();
    matches!(chars.next(), Some('a'..='z' | '0'..='9'))
        && text.len() <= 63
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}
