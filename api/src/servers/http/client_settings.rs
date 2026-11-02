//! Réglages et réinstallation des serveurs du propriétaire (ADR-0057) : les champs du prefab
//! marqués `visibility: customer`, prêts à afficher (libellés traduits), et la réinstallation
//! (fichiers du jeu remis à neuf, données gardées). Ni ressources, ni version du prefab : la
//! formule et le catalogue de l'extension les fixent.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use kuro_protocol::api::install::InstallMode;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::client::owned;
use super::servers::UpdateSettingsRequest;
use crate::http::error::{ApiError, Problem};
use crate::http::extract::{Json, Path};
use crate::iam::http::OwnerSession;
use crate::servers::application::owner_settings::{self, OwnerField, OwnerGroup};
use crate::servers::application::{self as servers, ServersState};
use kuro_prefab::manifest::{Editable, Section};

#[derive(Debug, Deserialize, IntoParams)]
pub struct LangQuery {
    /// Langue des libellés (`fr`, `en`…) ; à défaut l'anglais.
    #[serde(default = "english")]
    pub lang: String,
}

fn english() -> String {
    "en".to_owned()
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OwnerOptionDto {
    #[schema(value_type = Object)]
    pub value: Value,
    pub label: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OwnerFieldDto {
    pub key: String,
    /// `string`, `text`, `int`, `float`, `bool`, `enum`, `password`, `slider`.
    #[serde(rename = "type")]
    pub kind: String,
    pub label: String,
    pub help: Option<String>,
    #[schema(value_type = Option<Object>)]
    pub default: Option<Value>,
    pub required: bool,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    pub pattern: Option<String>,
    pub max_length: Option<usize>,
    pub options: Vec<OwnerOptionDto>,
    /// Faux : fixé au déploiement, affiché en lecture seule.
    pub editable: bool,
    /// Changer la valeur redémarre le serveur.
    pub restart: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OwnerGroupDto {
    /// `general` ou `advanced`.
    pub section: String,
    pub key: String,
    pub title: String,
    /// Nom d'icône Lucide.
    pub icon: Option<String>,
    pub fields: Vec<OwnerFieldDto>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OwnerSettingsDto {
    pub groups: Vec<OwnerGroupDto>,
    /// Valeurs actuelles des champs listés.
    #[schema(value_type = Object)]
    pub values: Map<String, Value>,
}

fn snake<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn field_dto(field: OwnerField) -> OwnerFieldDto {
    OwnerFieldDto {
        kind: snake(&field.field.kind),
        default: field.field.default,
        required: field.field.required,
        min: field.field.min,
        max: field.field.max,
        step: field.field.step,
        pattern: field.field.pattern,
        max_length: field.field.max_length,
        editable: field.field.editable == Editable::Always,
        restart: field.field.restart,
        options: field
            .options
            .into_iter()
            .map(|(value, label)| OwnerOptionDto { value, label })
            .collect(),
        key: field.key,
        label: field.label,
        help: field.help,
    }
}

fn group_dto(group: OwnerGroup) -> OwnerGroupDto {
    OwnerGroupDto {
        section: match group.section {
            Section::General => "general",
            Section::Advanced => "advanced",
        }
        .to_owned(),
        key: group.key,
        title: group.title,
        icon: group.icon,
        fields: group.fields.into_iter().map(field_dto).collect(),
    }
}

async fn settings_of(
    state: &ServersState,
    id: Uuid,
    lang: &str,
) -> Result<OwnerSettingsDto, ApiError> {
    let settings = owner_settings::get_for_owner(state, id, lang).await?;
    Ok(OwnerSettingsDto {
        groups: settings.groups.into_iter().map(group_dto).collect(),
        values: settings.values,
    })
}

/// Réglages du serveur ouverts au propriétaire (`visibility: customer`), libellés dans `lang`.
#[utoipa::path(get, path = "/api/client/v1/servers/{id}/settings", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), LangQuery),
    responses((status = 200, body = OwnerSettingsDto), (status = 401, body = Problem), (status = 404, body = Problem)))]
pub async fn get(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path(id): Path<Uuid>,
    Query(query): Query<LangQuery>,
) -> Result<Json<OwnerSettingsDto>, ApiError> {
    owned(&state, &session, id).await?;
    Ok(Json(settings_of(&state, id, &query.lang).await?))
}

/// Modifie des réglages ouverts au propriétaire, puis redéploie le serveur (redémarré s'il
/// tournait). Un champ hors de sa vue, ou fixé au déploiement : `422 setting_not_editable`.
#[utoipa::path(patch, path = "/api/client/v1/servers/{id}/settings", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), LangQuery),
    request_body = UpdateSettingsRequest,
    responses((status = 200, body = OwnerSettingsDto), (status = 401, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 422, body = Problem)))]
pub async fn update(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path(id): Path<Uuid>,
    Query(query): Query<LangQuery>,
    Json(request): Json<UpdateSettingsRequest>,
) -> Result<Json<OwnerSettingsDto>, ApiError> {
    owned(&state, &session, id).await?;
    owner_settings::update_for_owner(&state, &session.context, id, &request.settings).await?;
    Ok(Json(settings_of(&state, id, &query.lang).await?))
}

/// Réinstalle le serveur : étapes d'installation du prefab sur les données existantes (le nœud
/// l'arrête d'abord). L'état suit (`installing`, puis `offline`).
#[utoipa::path(post, path = "/api/client/v1/servers/{id}/reinstall", tag = "client-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 202), (status = 401, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 502, body = Problem)))]
pub async fn reinstall(
    State(state): State<ServersState>,
    session: OwnerSession,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    owned(&state, &session, id).await?;
    servers::reinstall(&state, &session.context, id, InstallMode::Reinstall).await?;
    Ok(StatusCode::ACCEPTED)
}
