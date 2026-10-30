//! Contenu des serveurs (ADR-0020, ADR-0029) : emplacements du prefab et leurs sources
//! (Modrinth, CurseForge), recherche et versions compatibles, installation avec les dépendances
//! obligatoires, mise à jour, activation, suppression. Le nœud télécharge et vérifie ; l'API
//! choisit les fichiers. En mode `reference` (ARK), le jeu télécharge lui-même : Kuro garde les
//! identifiants et redéploie le serveur.

use std::collections::{HashMap, HashSet, VecDeque};

use kuro_prefab::manifest::{ContentFieldType, ContentKind, Provider};
use kuro_prefab::{ContentPlan, ContentSource};
use kuro_protocol::ApiMessage;
use kuro_protocol::api::{ContentFile, ContentHash, ContentInstall, ContentRemove, ContentToggle};
use kuro_protocol::daemon::{ContentDone, ContentOutcome};
use serde_json::{Map, Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use super::deploy::{accepted, content_plans, deploy, record_failure};
use super::manage::{ensure_below_quota, ensure_not_suspended, ensure_ready};
use super::{ServersError, ServersState, get};
use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::content::{
    Category, Compat, ContentProvider, InstalledRef, Loader, ProviderError, ProviderVersion,
    SearchPage, SearchQuery, SearchSort, source_name,
};
use crate::error_code::ErrorCode;
use crate::nodes::domain::NodeId;
use crate::servers::domain::{NodeCommands, ServerState};
use crate::servers::infra::{
    ContentRecord, ContentRepository, NewContent, ServerRecord, ServerRepository,
};

/// Dépendances résolues au plus pour une installation.
const MAX_DEPENDENCIES: usize = 32;
const SEARCH_PAGE: u32 = 20;
/// Catégories d'une recherche au plus.
const MAX_CATEGORIES: usize = 10;
/// Valeurs d'un champ au plus, et caractères d'une valeur (ADR-0030).
const MAX_FIELD_VALUES: usize = 32;
const MAX_FIELD_VALUE_CHARS: usize = 128;

/// Valeurs saisies pour un contenu, vérifiées contre les champs de l'emplacement (ADR-0030) :
/// champ déclaré, une seule valeur pour `string`, bornées, sans caractère de contrôle (elles
/// finissent dans les fichiers de configuration du jeu). Rend `{ champ: [valeurs] }`.
fn checked_values(plan: &ContentPlan, input: &Map<String, Value>) -> Result<Value, ServersError> {
    let invalid = |field: &str, code| {
        ServersError::Invalid(crate::error_code::FieldError::new(
            format!("values.{field}"),
            code,
        ))
    };
    let mut out = Map::new();
    for (key, raw) in input {
        let field = plan
            .fields
            .get(key)
            .ok_or_else(|| invalid(key, ErrorCode::InvalidContentValue))?;
        let list: Vec<String> = match raw {
            Value::Null => Vec::new(),
            Value::String(text) => vec![text.clone()],
            Value::Array(items) => items
                .iter()
                .map(|item| item.as_str().map(str::to_owned))
                .collect::<Option<_>>()
                .ok_or_else(|| invalid(key, ErrorCode::InvalidContentValue))?,
            _ => return Err(invalid(key, ErrorCode::InvalidContentValue)),
        };
        let list: Vec<String> = list
            .iter()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .collect();
        let valid = list.len() <= MAX_FIELD_VALUES
            && (field.kind == ContentFieldType::List || list.len() <= 1)
            && list.iter().all(|value| {
                value.chars().count() <= MAX_FIELD_VALUE_CHARS
                    && !value.chars().any(char::is_control)
            });
        if !valid {
            return Err(invalid(key, ErrorCode::InvalidContentValue));
        }
        if !list.is_empty() {
            out.insert(key.clone(), json!(list));
        }
    }
    if let Some((key, _)) = plan
        .fields
        .iter()
        .find(|(key, field)| field.required && !out.contains_key(*key))
    {
        return Err(invalid(key, ErrorCode::ContentValueRequired));
    }
    Ok(Value::Object(out))
}

/// Filtres de la recherche, communs aux sources : ordre et catégories (identifiants donnés par
/// la source ; un résultat les a toutes, comme l'impose CurseForge). Le même cas d'usage sert
/// l'espace admin et, plus tard, l'espace client.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchFilters {
    pub sort: SearchSort,
    pub categories: Vec<String>,
}

impl SearchFilters {
    /// Identifiant de catégorie : nom (Modrinth) ou numéro (CurseForge), jamais à échapper.
    fn validated(&self) -> Result<Vec<String>, ServersError> {
        let valid = self.categories.len() <= MAX_CATEGORIES
            && self.categories.iter().all(|c| {
                !c.is_empty()
                    && c.len() <= 64
                    && c.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            });
        if !valid {
            return Err(ServersError::Unprocessable(ErrorCode::InvalidParameter));
        }
        Ok(self.categories.clone())
    }
}

/// Emplacement de contenu d'un serveur, ses sources et ce qui empêche de les utiliser.
#[derive(Debug, Clone)]
pub struct Slot {
    pub plan: ContentPlan,
    /// Sources déclarées par le prefab, dans son ordre (hors envoi de fichiers).
    pub sources: Vec<SlotSource>,
    /// Version stable à proposer quand la version du jeu n'est pas figée.
    pub latest_game_version: Option<String>,
}

/// Source d'un emplacement : sa compatibilité, ou ce qui l'empêche d'être utilisée.
#[derive(Debug, Clone)]
pub struct SlotSource {
    pub provider: Provider,
    pub compat: Option<Compat>,
    pub blocked: Option<ErrorCode>,
}

impl Slot {
    /// Source proposée par défaut : la première utilisable, sinon la première.
    pub fn default_source(&self) -> Option<&SlotSource> {
        self.sources
            .iter()
            .find(|source| source.blocked.is_none())
            .or_else(|| self.sources.first())
    }

    /// Ce qui empêche d'utiliser l'emplacement, d'après sa source par défaut.
    pub fn blocked(&self) -> Option<ErrorCode> {
        match self.default_source() {
            Some(source) => source.blocked,
            None => Some(ErrorCode::ContentNotSupported),
        }
    }

    /// Le prefab accepte des fichiers envoyés depuis l'ordinateur, déposés dans le dossier de
    /// l'emplacement (jamais en mode `reference`).
    pub fn accepts_upload(&self) -> bool {
        accepts_upload(&self.plan)
    }

    pub fn compat(&self) -> Option<&Compat> {
        self.default_source()
            .and_then(|source| source.compat.as_ref())
    }

    /// Mode `reference` : le jeu télécharge lui-même ; Kuro ne garde que les identifiants.
    pub fn is_reference(&self) -> bool {
        self.plan.dir.is_none()
    }
}

/// Contenu installé et ce que l'on sait de lui.
#[derive(Debug, Clone)]
pub struct Installed {
    pub items: Vec<ContentRecord>,
    /// (contenu exigé, contenu qui l'exige).
    pub links: Vec<(Uuid, Uuid)>,
    /// Contenu → paquets extraits (mode `extract`), dans l'ordre de l'archive.
    pub packages: HashMap<Uuid, Vec<crate::servers::infra::ContentPackage>>,
    /// Contenus en attente du prochain démarrage du serveur.
    pub pending: HashSet<Uuid>,
    /// Contenu → version compatible plus récente.
    pub updates: HashMap<Uuid, ProviderVersion>,
}

fn project_type(kind: ContentKind) -> &'static str {
    match kind {
        ContentKind::Mods => "mod",
        ContentKind::Modpacks => "modpack",
        ContentKind::Resourcepacks => "resourcepack",
        ContentKind::Plugins => "plugin",
        ContentKind::Maps => "map",
        ContentKind::Addons => "addon",
    }
}

/// Version du jeu figée (`1.21.4`), seule utilisable pour choisir un mod.
fn is_pinned(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    (2..=3).contains(&parts.len())
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

fn provider_error(error: ProviderError) -> ServersError {
    match error {
        ProviderError::NotFound => ServersError::Missing(ErrorCode::ContentNotFound),
        ProviderError::NotConfigured => {
            ServersError::Unprocessable(ErrorCode::ContentSourceNotConfigured)
        }
        ProviderError::Unauthorized => ServersError::Daemon {
            code: ErrorCode::ContentSourceUnauthorized,
            message: "the content source refused the api key".to_owned(),
        },
        ProviderError::Unavailable(message) => ServersError::Daemon {
            code: ErrorCode::ProviderUnavailable,
            message,
        },
    }
}

mod catalog;
mod changes;
mod events;
mod install;
mod modpack;
mod slots;
mod upload;

use install::prefilled;
use modpack::game_root;
use slots::{accepts_upload, source_of, usable_slot};

pub use catalog::{categories, project, search, versions};
pub use changes::{orphans, remove, set_values, toggle};
pub use events::{record_done, redeploy_registered};
pub use install::{install, update};
pub use modpack::{install_modpack, modpack_search, modpack_versions};
pub use slots::{installed, loaders, slots};
pub use upload::upload;

/// Serveur en marche : un changement de contenu attend son prochain démarrage (un jeu ne
/// recharge ni ses mods ni ses packs à chaud) ; il n'est pas redémarré pour autant.
fn is_live(server: &ServerRecord) -> bool {
    matches!(server.state.as_str(), "running" | "starting")
}

/// Après un changement de contenu inscrit (référence, paquets extraits) : un serveur arrêté est
/// redéployé ; un serveur en marche reçoit sa configuration sans être redémarré.
async fn apply_registered(
    state: &ServersState,
    server_id: Uuid,
    live: bool,
) -> Result<(), ServersError> {
    if !live {
        return redeploy(state, server_id).await;
    }
    if let Err(error) = super::deploy::deploy_deferred(state, server_id).await {
        record_failure(state, server_id, &error).await;
        return Err(error);
    }
    Ok(())
}

/// Redéploie après un changement du contenu en mode `reference` ; un échec passe le serveur en
/// `failed`, avec sa raison.
async fn redeploy(state: &ServersState, server_id: Uuid) -> Result<(), ServersError> {
    if let Err(error) = deploy(state, server_id, false).await {
        record_failure(state, server_id, &error).await;
        return Err(error);
    }
    Ok(())
}

fn refusal(error: &ServersError) -> &'static str {
    match error {
        ServersError::Conflict(code) | ServersError::Unprocessable(code) => code.as_str(),
        ServersError::Daemon { code, .. } => code.as_str(),
        _ => ErrorCode::InternalError.as_str(),
    }
}

async fn plan_of(
    state: &ServersState,
    server: &ServerRecord,
    slot: &str,
) -> Result<ContentPlan, ServersError> {
    content_plans(state, server)
        .await?
        .into_iter()
        .find(|plan| plan.slot == slot)
        .ok_or(ServersError::Missing(ErrorCode::ContentSlotNotFound))
}
