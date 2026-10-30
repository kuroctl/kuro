//! Valeurs des réglages d'un serveur, validées contre les champs du prefab.
//!
//! Sert à l'import (valeurs par défaut) et à chaque saisie côté API : une erreur est rattachée
//! à son champ, avec une raison stable que le panel traduit.

use std::collections::BTreeMap;

use regex::Regex;
use serde_json::{Map, Value};

use crate::manifest::{Editable, Field, FieldType, Manifest};

/// Champ → valeur. Les secrets y figurent aussi : c'est l'appelant qui les met à part.
pub type Values = BTreeMap<String, Value>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reason {
    UnknownField,
    Type,
    Required,
    Min,
    Max,
    Pattern,
    MaxLength,
    Options,
    /// Retour à la ligne ou caractère de contrôle dans une valeur sur une ligne : il pourrait
    /// injecter une ligne dans un fichier de configuration.
    ControlCharacter,
    /// Champ `editable: deploy` modifié après le déploiement.
    NotEditable,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnknownField => "unknown_field",
            Self::Type => "type",
            Self::Required => "required",
            Self::Min => "min",
            Self::Max => "max",
            Self::Pattern => "pattern",
            Self::MaxLength => "max_length",
            Self::Options => "options",
            Self::ControlCharacter => "control_character",
            Self::NotEditable => "not_editable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingError {
    pub field: String,
    pub reason: Reason,
}

/// Vérifie une valeur contre son champ (type, bornes, motif, options).
pub fn check_value(field: &Field, value: &Value) -> Result<(), Reason> {
    match field.kind {
        FieldType::Bool => value.as_bool().map(|_| ()).ok_or(Reason::Type),
        FieldType::Int => {
            let number = value.as_i64().ok_or(Reason::Type)?;
            bounds(field, number as f64)
        }
        FieldType::Float | FieldType::Slider => {
            let number = value.as_f64().ok_or(Reason::Type)?;
            bounds(field, number)
        }
        FieldType::Enum => {
            let options = field.options.as_deref().unwrap_or_default();
            if options.contains(value) {
                Ok(())
            } else {
                Err(Reason::Options)
            }
        }
        FieldType::String | FieldType::Password | FieldType::Secret | FieldType::Text => {
            let text = value.as_str().ok_or(Reason::Type)?;
            let multiline = field.kind == FieldType::Text;
            if text
                .chars()
                .any(|c| c.is_control() && !(multiline && (c == '\n' || c == '\t')))
            {
                return Err(Reason::ControlCharacter);
            }
            if field
                .max_length
                .is_some_and(|max| text.chars().count() > max)
            {
                return Err(Reason::MaxLength);
            }
            if let Some(pattern) = &field.pattern {
                let matches = Regex::new(pattern).is_ok_and(|re| re.is_match(text));
                if !matches {
                    return Err(Reason::Pattern);
                }
            }
            Ok(())
        }
    }
}

fn bounds(field: &Field, number: f64) -> Result<(), Reason> {
    if field.min.is_some_and(|min| number < min) {
        return Err(Reason::Min);
    }
    if field.max.is_some_and(|max| number > max) {
        return Err(Reason::Max);
    }
    Ok(())
}

/// Valeurs par défaut déclarées par le prefab.
pub fn defaults(manifest: &Manifest) -> Values {
    manifest
        .settings
        .fields()
        .filter_map(|f| f.field.default.clone().map(|v| (f.key.to_owned(), v)))
        .collect()
}

/// Secrets à générer au déploiement : champ → longueur.
pub fn secrets_to_generate(manifest: &Manifest) -> Vec<(String, usize)> {
    manifest
        .settings
        .fields()
        .filter_map(|f| f.field.generate.map(|g| (f.key.to_owned(), g.length)))
        .collect()
}

/// Applique une saisie (complète ou partielle) aux valeurs actuelles.
///
/// `current` vaut `None` au déploiement : on part des valeurs par défaut et les champs
/// `editable: deploy` sont libres. `null` remet un champ à sa valeur par défaut.
/// Les secrets `generate` manquants ne sont pas une erreur : l'appelant les génère ensuite.
pub fn apply(
    manifest: &Manifest,
    current: Option<&Values>,
    input: &Map<String, Value>,
) -> Result<Values, Vec<SettingError>> {
    let defaults = defaults(manifest);
    let mut values = current.cloned().unwrap_or_else(|| defaults.clone());
    let mut errors = Vec::new();
    let mut fail = |field: &str, reason| {
        errors.push(SettingError {
            field: field.to_owned(),
            reason,
        });
    };

    for (key, value) in input {
        let Some(field) = manifest.settings.field(key) else {
            fail(key, Reason::UnknownField);
            continue;
        };
        let next = if value.is_null() {
            defaults.get(key).cloned()
        } else {
            Some(value.clone())
        };
        if current.is_some()
            && field.editable == Editable::Deploy
            && values.get(key) != next.as_ref()
        {
            fail(key, Reason::NotEditable);
            continue;
        }
        match next {
            Some(next) => match check_value(field, &next) {
                Ok(()) => {
                    values.insert(key.clone(), next);
                }
                Err(reason) => fail(key, reason),
            },
            None => {
                values.remove(key);
            }
        }
    }

    for f in manifest.settings.fields() {
        let missing = values.get(f.key).is_none_or(|v| v.as_str() == Some(""));
        if f.field.required && missing && f.field.generate.is_none() {
            fail(f.key, Reason::Required);
        }
    }

    if errors.is_empty() {
        Ok(values)
    } else {
        Err(errors)
    }
}

/// Réglages passés à une nouvelle version (ADR-0024).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Migrated {
    pub values: Values,
    /// Ancienne clé → nouvelle.
    pub renamed: Vec<(String, String)>,
    /// Clés retirées : par une migration, ou inconnues de la nouvelle version.
    pub removed: Vec<String>,
    /// Nouveaux champs, à leur valeur par défaut.
    pub added: Vec<String>,
}

/// Passe des réglages enregistrés sous la version `from` à `target` : migrations dont la
/// contrainte couvre `from` (dans l'ordre, `rename` puis `remove`), clés inconnues retirées,
/// nouveaux champs à leur valeur par défaut, puis chaque valeur revérifiée. Une valeur devenue
/// invalide est une erreur : la mise à jour est alors bloquée.
pub fn migrate(
    from: &semver::Version,
    target: &Manifest,
    current: &Values,
) -> Result<Migrated, Vec<SettingError>> {
    let mut values = current.clone();
    let mut renamed = Vec::new();
    let mut removed = Vec::new();
    for migration in &target.migrations {
        let applies = semver::VersionReq::parse(&migration.from).is_ok_and(|req| req.matches(from));
        if !applies {
            continue;
        }
        for (old, new) in &migration.rename {
            if let Some(value) = values.remove(old) {
                values.insert(new.clone(), value);
                renamed.push((old.clone(), new.clone()));
            }
        }
        for key in &migration.remove {
            if values.remove(key).is_some() {
                removed.push(key.clone());
            }
        }
    }
    let unknown: Vec<String> = values
        .keys()
        .filter(|key| target.settings.field(key).is_none())
        .cloned()
        .collect();
    for key in unknown {
        values.remove(&key);
        removed.push(key);
    }

    let mut added = Vec::new();
    let mut errors = Vec::new();
    for f in target.settings.fields() {
        match values.get(f.key) {
            Some(value) => {
                if let Err(reason) = check_value(f.field, value) {
                    errors.push(SettingError {
                        field: f.key.to_owned(),
                        reason,
                    });
                }
            }
            None => {
                if let Some(default) = &f.field.default {
                    values.insert(f.key.to_owned(), default.clone());
                    added.push(f.key.to_owned());
                } else if f.field.generate.is_some() {
                    // Secret généré par l'appelant, comme au déploiement.
                    added.push(f.key.to_owned());
                } else if f.field.required {
                    errors.push(SettingError {
                        field: f.key.to_owned(),
                        reason: Reason::Required,
                    });
                }
            }
        }
    }
    if errors.is_empty() {
        Ok(Migrated {
            values,
            renamed,
            removed,
            added,
        })
    } else {
        Err(errors)
    }
}
