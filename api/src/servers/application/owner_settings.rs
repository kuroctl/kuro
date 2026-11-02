//! Réglages d'un serveur vus et modifiés par son propriétaire (ADR-0057) : seuls les champs du
//! prefab marqués `visibility: customer` existent pour lui, jamais un secret généré. Libellés,
//! aides et options sont ceux des traductions du prefab, dans la langue demandée.

use serde_json::{Map, Value};
use uuid::Uuid;

use super::{ServerView, ServersError, ServersState, get, manage, merged_values};
use crate::audit::domain::AuditContext;
use crate::error_code::{ErrorCode, FieldError};
use crate::prefabs::application as prefabs;
use kuro_prefab::manifest::{Editable, Field, FieldType, Section, Visibility};

/// Champ visible du propriétaire, avec ses textes traduits.
#[derive(Debug, Clone)]
pub struct OwnerField {
    pub key: String,
    pub field: Field,
    pub label: String,
    pub help: Option<String>,
    /// Option → libellé.
    pub options: Vec<(Value, String)>,
}

#[derive(Debug, Clone)]
pub struct OwnerGroup {
    pub section: Section,
    pub key: String,
    pub title: String,
    pub icon: Option<String>,
    pub fields: Vec<OwnerField>,
}

/// Réglages du propriétaire : groupes (dans l'ordre du prefab) et valeurs actuelles.
pub struct OwnerSettings {
    pub groups: Vec<OwnerGroup>,
    pub values: Map<String, Value>,
}

fn for_owner(field: &Field) -> bool {
    field.visibility == Visibility::Customer && field.kind != FieldType::Secret
}

/// Traductions de la langue demandée, sinon l'anglais, sinon la première.
fn messages<'a>(
    locales: &'a std::collections::BTreeMap<String, Value>,
    lang: &str,
) -> Option<&'a Value> {
    locales
        .get(lang)
        .or_else(|| locales.get("en"))
        .or_else(|| locales.values().next())
}

fn text(messages: Option<&Value>, pointer: &str) -> Option<String> {
    messages?
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Option d'un champ `enum`, en clé de traduction (`"normal"`, `"1"`).
fn option_key(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned)
}

pub async fn get_for_owner(
    state: &ServersState,
    id: Uuid,
    lang: &str,
) -> Result<OwnerSettings, ServersError> {
    let server = get(state, id).await?.server;
    let package = prefabs::package(&state.pool, &server.prefab_id, &server.prefab_version).await?;
    let messages = messages(&package.locales, lang);
    let settings = &package.manifest.settings;
    let mut groups = Vec::new();
    for (section, map) in [
        (Section::General, &settings.general),
        (Section::Advanced, &settings.advanced),
    ] {
        for (group_key, group) in map {
            let fields: Vec<OwnerField> = group
                .fields
                .iter()
                .filter(|(_, field)| for_owner(field))
                .map(|(key, field)| OwnerField {
                    key: key.clone(),
                    label: text(messages, &format!("/fields/{key}/label"))
                        .unwrap_or_else(|| key.clone()),
                    help: text(messages, &format!("/fields/{key}/help")),
                    options: field
                        .options
                        .iter()
                        .flatten()
                        .map(|option| {
                            let name = option_key(option);
                            let label = text(messages, &format!("/fields/{key}/options/{name}"))
                                .unwrap_or_else(|| name.clone());
                            (option.clone(), label)
                        })
                        .collect(),
                    field: field.clone(),
                })
                .collect();
            if fields.is_empty() {
                continue;
            }
            groups.push(OwnerGroup {
                section,
                key: group_key.clone(),
                title: text(messages, &format!("/groups/{group_key}/title"))
                    .unwrap_or_else(|| group_key.clone()),
                icon: group.icon.clone(),
                fields,
            });
        }
    }
    let all = merged_values(state, &server)?;
    let values = groups
        .iter()
        .flat_map(|group| &group.fields)
        .filter_map(|field| {
            all.get(&field.key)
                .or(field.field.default.as_ref())
                .map(|value| (field.key.clone(), value.clone()))
        })
        .collect();
    Ok(OwnerSettings { groups, values })
}

/// Modifie des réglages du propriétaire : un champ qu'il ne voit pas, ou fixé au déploiement, est
/// refusé (`settings.<clé>`, `setting_not_editable`) ; le reste suit le cas d'usage de l'admin
/// (validation, redéploiement).
pub async fn update_for_owner(
    state: &ServersState,
    context: &AuditContext,
    id: Uuid,
    input: &Map<String, Value>,
) -> Result<ServerView, ServersError> {
    let server = get(state, id).await?.server;
    let package = prefabs::package(&state.pool, &server.prefab_id, &server.prefab_version).await?;
    let refused: Vec<FieldError> = input
        .keys()
        .filter(|key| {
            package
                .manifest
                .settings
                .field(key)
                .is_none_or(|field| !for_owner(field) || field.editable == Editable::Deploy)
        })
        .map(|key| FieldError::new(format!("settings.{key}"), ErrorCode::SettingNotEditable))
        .collect();
    if !refused.is_empty() {
        return Err(ServersError::InvalidSettings(refused));
    }
    manage::update_settings(state, context, id, input).await
}
