//! Contenu d'un serveur (ADR-0020, ADR-0029) : emplacements et leurs sources, installés,
//! recherche, versions compatibles, installation, mise à jour, activation, suppression.

use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::content::{
    Category, ProjectHit, ProviderVersion, SearchSort, project_page, source_name,
};
use crate::http::error::{ApiError, ErrorCode, Problem};
use crate::http::extract::{Json, Path, Query};
use crate::iam::application::IamState;
use crate::iam::domain::Permission;
use crate::iam::http::AdminSession;
use crate::servers::application::ServersState;
use crate::servers::application::content::{self, Installed, SearchFilters, Slot};
use crate::servers::infra::ContentRecord;

#[derive(Debug, Serialize, ToSchema)]
pub struct ContentSlotDto {
    /// Clé de l'emplacement dans le prefab (`mods`, `plugins`).
    pub slot: String,
    /// `mods`, `plugins`, `modpacks`…
    pub kind: String,
    /// Loader et version du jeu, rendus avec les réglages du serveur.
    pub loader: Option<String>,
    pub game_version: Option<String>,
    /// Ce qui empêche d'ajouter du contenu depuis la source par défaut (loader vanilla, version
    /// du jeu non figée, clé d'API manquante).
    pub blocked: Option<ErrorCode>,
    /// Version stable à proposer quand la version du jeu n'est pas figée.
    pub latest_game_version: Option<String>,
    /// Sources de l'emplacement, la source par défaut en premier utilisable.
    pub sources: Vec<ContentSlotSourceDto>,
    /// `reference` : le jeu télécharge lui-même, Kuro garde les identifiants (ARK) ;
    /// `download` : le nœud dépose les fichiers.
    pub mode: String,
    /// Le prefab accepte des fichiers envoyés depuis l'ordinateur.
    pub upload: bool,
    /// Valeurs à saisir pour chaque contenu (ADR-0030), libellées par les traductions du
    /// prefab (`content.<emplacement>.fields.<champ>`).
    pub fields: Vec<ContentFieldDto>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ContentFieldDto {
    pub key: String,
    /// `string` (une valeur) ou `list`.
    #[serde(rename = "type")]
    pub kind: String,
    pub required: bool,
    /// Le prefab sait trouver des valeurs dans la description d'un projet (`prefill`).
    pub prefill: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ContentSlotSourceDto {
    /// `modrinth`, `curseforge`.
    pub provider: String,
    pub blocked: Option<ErrorCode>,
}

impl From<Slot> for ContentSlotDto {
    fn from(slot: Slot) -> Self {
        // Loader et version affichés : ceux de la source par défaut.
        let provider = slot.default_source().map(|s| s.provider);
        let decl = slot
            .plan
            .providers
            .iter()
            .find(|p| Some(p.provider) == provider);
        Self {
            kind: serde_json::to_value(slot.plan.kind)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default(),
            loader: decl.and_then(|s| s.loader.clone()),
            game_version: decl.and_then(|s| s.game_version.clone()),
            blocked: slot.blocked(),
            upload: slot.accepts_upload(),
            fields: slot
                .plan
                .fields
                .iter()
                .map(|(key, field)| ContentFieldDto {
                    key: key.clone(),
                    kind: match field.kind {
                        kuro_prefab::manifest::ContentFieldType::String => "string",
                        kuro_prefab::manifest::ContentFieldType::List => "list",
                    }
                    .to_owned(),
                    required: field.required,
                    prefill: field.prefill.is_some(),
                })
                .collect(),
            mode: if slot.is_reference() {
                "reference"
            } else {
                "download"
            }
            .to_owned(),
            sources: slot
                .sources
                .iter()
                .map(|s| ContentSlotSourceDto {
                    provider: source_name(s.provider).to_owned(),
                    blocked: s.blocked,
                })
                .collect(),
            slot: slot.plan.slot,
            latest_game_version: slot.latest_game_version,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContentStateDto {
    Installing,
    Installed,
    Failed,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ContentUpdateDto {
    pub version_id: String,
    pub version_name: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ContentDto {
    pub id: Uuid,
    pub name: String,
    /// Page du projet chez sa source (description) ; absente pour un fichier envoyé.
    pub page_url: Option<String>,
    /// `modrinth`, `upload`, `modpack`.
    pub source: String,
    pub project_id: Option<String>,
    pub version_id: Option<String>,
    pub version_name: Option<String>,
    pub icon_url: Option<String>,
    pub file_name: String,
    pub size_bytes: Option<i64>,
    pub enabled: bool,
    pub state: ContentStateDto,
    pub failure: Option<String>,
    /// `file`, ou `modpack` (sans fichier propre : ses fichiers le référencent).
    pub kind: String,
    /// Modpack dont ce fichier fait partie : il part avec lui.
    pub modpack_id: Option<Uuid>,
    /// Valeurs saisies pour les champs de l'emplacement : champ → liste.
    #[schema(value_type = Object)]
    pub values: serde_json::Value,
    /// Contenus qui exigent celui-ci : il ne peut pas être supprimé seul.
    pub required_by: Vec<Uuid>,
    /// Dépendances supprimées avec lui si plus rien d'autre ne les exige.
    pub orphans: Vec<Uuid>,
    /// Version compatible plus récente.
    pub update: Option<ContentUpdateDto>,
    /// Paquets extraits (mode `extract`, add-ons Bedrock), dans l'ordre de l'archive.
    pub packages: Vec<ContentPackageDto>,
    /// Changé pendant que le serveur tournait : appliqué à son prochain démarrage.
    pub pending: bool,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
}

/// Paquet extrait d'un contenu.
#[derive(Debug, Serialize, ToSchema)]
pub struct ContentPackageDto {
    /// Dossier choisi d'après sa description (`/data/behavior_packs`), et nom du sien dedans.
    pub dir: String,
    pub name: String,
    /// Nom et version lus dans sa description, s'il en a (`header.name`, `header.version`).
    pub title: Option<String>,
    pub version: Option<String>,
}

/// Nom et version d'un paquet, d'après les champs usuels de sa description (Bedrock :
/// `header.name`, `header.version` en liste de nombres).
fn package_dto(package: &crate::servers::infra::ContentPackage) -> ContentPackageDto {
    let header = &package.description["header"];
    let version = match &header["version"] {
        serde_json::Value::Array(parts) => Some(
            parts
                .iter()
                .map(|part| part.to_string().trim_matches('"').to_owned())
                .collect::<Vec<_>>()
                .join("."),
        ),
        serde_json::Value::String(text) => Some(text.clone()),
        _ => None,
    };
    ContentPackageDto {
        dir: package.dir.clone(),
        name: package.name.clone(),
        title: header["name"].as_str().map(str::to_owned),
        version,
    }
}

fn dto(item: ContentRecord, installed: Option<&Installed>) -> ContentDto {
    let pending = installed.is_some_and(|all| all.pending.contains(&item.id));
    let packages = installed
        .and_then(|all| all.packages.get(&item.id))
        .map(|packages| packages.iter().map(package_dto).collect())
        .unwrap_or_default();
    let (required_by, orphans, update) = match installed {
        Some(all) => (
            all.links
                .iter()
                .filter(|(dep, by)| *dep == item.id && *by != item.id)
                .map(|(_, by)| *by)
                .collect(),
            content::orphans(item.id, &all.links),
            all.updates.get(&item.id).map(|v| ContentUpdateDto {
                version_id: v.id.clone(),
                version_name: v.version_number.clone(),
            }),
        ),
        None => (Vec::new(), Vec::new(), None),
    };
    ContentDto {
        page_url: item
            .project_id
            .as_deref()
            .and_then(|project| project_page(&item.source, project)),
        id: item.id,
        name: item.name,
        source: item.source,
        project_id: item.project_id,
        version_id: item.version_id,
        version_name: item.version_name,
        icon_url: item.icon_url,
        file_name: item.file_name,
        size_bytes: item.size_bytes,
        enabled: item.enabled,
        state: match item.state.as_str() {
            "installing" => ContentStateDto::Installing,
            "installed" => ContentStateDto::Installed,
            _ => ContentStateDto::Failed,
        },
        failure: item.failure,
        kind: item.kind,
        modpack_id: item.modpack_id,
        values: item.field_values,
        required_by,
        orphans,
        update,
        packages,
        pending,
        created_at: item.created_at,
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ContentListDto {
    pub items: Vec<ContentDto>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProjectHitDto {
    pub project_id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub author: String,
    pub icon_url: Option<String>,
    pub downloads: u64,
    /// Page du projet chez la source.
    pub page_url: Option<String>,
    /// Faux : l'auteur interdit le téléchargement par des tiers (CurseForge), le panel renvoie
    /// vers la page du projet.
    pub download_allowed: bool,
}

impl From<ProjectHit> for ProjectHitDto {
    fn from(hit: ProjectHit) -> Self {
        Self {
            page_url: hit.page_url,
            download_allowed: hit.download_allowed,
            project_id: hit.project_id,
            slug: hit.slug,
            title: hit.title,
            description: hit.description,
            author: hit.author,
            icon_url: hit.icon_url,
            downloads: hit.downloads,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ContentSearchDto {
    pub hits: Vec<ProjectHitDto>,
    pub total: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ContentVersionDto {
    pub id: String,
    pub name: String,
    pub version_number: String,
    pub published: String,
    pub loaders: Vec<String>,
    pub game_versions: Vec<String>,
    pub file_name: Option<String>,
    pub size: Option<u64>,
    /// Projets exigés par cette version, installés avec elle s'ils manquent.
    pub required_projects: Vec<String>,
}

impl From<ProviderVersion> for ContentVersionDto {
    fn from(version: ProviderVersion) -> Self {
        let file = version.primary_file().cloned();
        Self {
            required_projects: version
                .dependencies
                .iter()
                .filter(|d| d.required)
                .filter_map(|d| d.project_id.clone())
                .collect(),
            id: version.id,
            name: version.name,
            version_number: version.version_number,
            published: version.published,
            loaders: version.loaders,
            game_versions: version.game_versions,
            file_name: file.as_ref().map(|f| f.file_name.clone()),
            size: file.map(|f| f.size),
        }
    }
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct SearchParams {
    /// Texte recherché ; vide : les plus populaires.
    pub q: Option<String>,
    pub offset: Option<u32>,
    /// Source (`modrinth`, `curseforge`) ; absente : celle par défaut de l'emplacement.
    pub source: Option<String>,
    /// `relevance` (par défaut), `downloads`, `updated`, `newest`.
    pub sort: Option<String>,
    /// Catégories de la source, séparées par des virgules (`GET …/categories`) : un résultat les
    /// a toutes. Dix au plus.
    pub categories: Option<String>,
}

impl SearchParams {
    fn filters(&self) -> Result<SearchFilters, ApiError> {
        let sort = match self.sort.as_deref() {
            None | Some("") => SearchSort::default(),
            Some(text) => SearchSort::parse(text).ok_or_else(|| {
                Problem::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    ErrorCode::InvalidParameter,
                )
            })?,
        };
        let categories = self
            .categories
            .as_deref()
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(str::to_owned)
            .collect();
        Ok(SearchFilters { sort, categories })
    }
}

/// Catégorie de contenu d'une source, à passer en filtre de recherche.
#[derive(Debug, Serialize, ToSchema)]
pub struct ContentCategoryDto {
    /// Identifiant à renvoyer dans `categories`.
    pub id: String,
    pub name: String,
    /// Icône SVG (Modrinth), à afficher comme masque CSS.
    pub icon_svg: Option<String>,
    /// Icône en image (CurseForge).
    pub icon_url: Option<String>,
}

impl From<Category> for ContentCategoryDto {
    fn from(category: Category) -> Self {
        Self {
            id: category.id,
            name: category.name,
            icon_svg: category.icon_svg,
            icon_url: category.icon_url,
        }
    }
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct SourceParams {
    /// Source (`modrinth`, `curseforge`) ; absente : celle par défaut de l'emplacement.
    pub source: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct InstallContentRequest {
    pub version_id: String,
    /// Source de la version ; absente : celle par défaut de l'emplacement.
    #[serde(default)]
    pub source: Option<String>,
    /// Valeurs des champs de l'emplacement : chaîne, ou liste pour un champ `list`.
    #[serde(default)]
    #[schema(value_type = Object)]
    pub values: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ContentValuesRequest {
    /// Valeurs des champs de l'emplacement ; un champ absent est vidé.
    #[schema(value_type = Object)]
    pub values: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ToggleContentRequest {
    pub enabled: bool,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct RemoveParams {
    /// Supprimer aussi les dépendances que plus rien n'exige.
    pub with_orphans: Option<bool>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RemovedContentDto {
    pub removed: Vec<Uuid>,
}

/// Emplacements de contenu du serveur. Permission : `servers.content.read`.
#[utoipa::path(get, path = "/api/admin/v1/servers/{id}/content", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [ContentSlotDto]), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn slots(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<ContentSlotDto>>, ApiError> {
    session
        .require(&iam, Permission::ServersContentRead)
        .await?;
    let slots = content::slots(&state, id).await?;
    Ok(Json(slots.into_iter().map(Into::into).collect()))
}

/// Contenu installé d'un emplacement, avec les mises à jour disponibles.
/// Permission : `servers.content.read`.
#[utoipa::path(get, path = "/api/admin/v1/servers/{id}/content/{slot}", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("slot" = String, Path)),
    responses((status = 200, body = ContentListDto), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn list(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path((id, slot)): Path<(Uuid, String)>,
) -> Result<Json<ContentListDto>, ApiError> {
    session
        .require(&iam, Permission::ServersContentRead)
        .await?;
    let installed = content::installed(&state, id, &slot).await?;
    let items = installed
        .items
        .clone()
        .into_iter()
        .map(|item| dto(item, Some(&installed)))
        .collect();
    Ok(Json(ContentListDto { items }))
}

/// Recherche chez le provider, filtrée par loader et version du jeu du serveur.
/// Permission : `servers.content.read`.
#[utoipa::path(get, path = "/api/admin/v1/servers/{id}/content/{slot}/search", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("slot" = String, Path), SearchParams),
    responses((status = 200, body = ContentSearchDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 422, description = "Emplacement inutilisable (loader, version du jeu)", body = Problem),
        (status = 502, body = Problem)))]
pub async fn search(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path((id, slot)): Path<(Uuid, String)>,
    Query(params): Query<SearchParams>,
) -> Result<Json<ContentSearchDto>, ApiError> {
    session
        .require(&iam, Permission::ServersContentRead)
        .await?;
    let filters = params.filters()?;
    let page = content::search(
        &state,
        id,
        &slot,
        params.source.as_deref(),
        params.q.as_deref().unwrap_or(""),
        params.offset.unwrap_or(0),
        &filters,
    )
    .await?;
    Ok(Json(ContentSearchDto {
        total: page.total,
        hits: page.hits.into_iter().map(Into::into).collect(),
    }))
}

/// Fiche d'un projet, avec les valeurs proposées pour les champs de l'emplacement.
#[derive(Debug, Serialize, ToSchema)]
pub struct ContentProjectDto {
    pub project_id: String,
    pub title: String,
    pub icon_url: Option<String>,
    /// Champ → valeurs trouvées dans la description (`prefill` du prefab), à faire vérifier.
    #[schema(value_type = Object)]
    pub prefill: serde_json::Value,
}

/// Fiche d'un projet chez la source, avec les valeurs proposées pour les champs de
/// l'emplacement d'après sa description. Permission : `servers.content.read`.
#[utoipa::path(get, path = "/api/admin/v1/servers/{id}/content/{slot}/projects/{project}", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("slot" = String, Path), ("project" = String, Path), SourceParams),
    responses((status = 200, body = ContentProjectDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn project(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path((id, slot, project)): Path<(Uuid, String, String)>,
    Query(params): Query<SourceParams>,
) -> Result<Json<ContentProjectDto>, ApiError> {
    session
        .require(&iam, Permission::ServersContentRead)
        .await?;
    let (project, prefill) =
        content::project(&state, id, &slot, params.source.as_deref(), &project).await?;
    Ok(Json(ContentProjectDto {
        project_id: project.id,
        title: project.title,
        icon_url: project.icon_url,
        prefill,
    }))
}

/// Catégories d'une source pour cet emplacement, à proposer en filtres de recherche.
/// Permission : `servers.content.read`.
#[utoipa::path(get, path = "/api/admin/v1/servers/{id}/content/{slot}/categories", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("slot" = String, Path), SourceParams),
    responses((status = 200, body = [ContentCategoryDto]), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn categories(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path((id, slot)): Path<(Uuid, String)>,
    Query(params): Query<SourceParams>,
) -> Result<Json<Vec<ContentCategoryDto>>, ApiError> {
    session
        .require(&iam, Permission::ServersContentRead)
        .await?;
    let categories = content::categories(&state, id, &slot, params.source.as_deref()).await?;
    Ok(Json(categories.into_iter().map(Into::into).collect()))
}

/// Versions compatibles d'un projet, de la plus récente à la plus ancienne.
/// Permission : `servers.content.read`.
#[utoipa::path(get, path = "/api/admin/v1/servers/{id}/content/{slot}/projects/{project}/versions", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("slot" = String, Path), ("project" = String, Path), SourceParams),
    responses((status = 200, body = [ContentVersionDto]), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn versions(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path((id, slot, project)): Path<(Uuid, String, String)>,
    Query(params): Query<SourceParams>,
) -> Result<Json<Vec<ContentVersionDto>>, ApiError> {
    session
        .require(&iam, Permission::ServersContentRead)
        .await?;
    let versions = content::versions(&state, id, &slot, params.source.as_deref(), &project).await?;
    Ok(Json(versions.into_iter().map(Into::into).collect()))
}

/// Installe une version et ses dépendances obligatoires ; le nœud télécharge et vérifie,
/// l'état suit (`installing` → `installed` ou `failed`). Pris en compte au redémarrage du jeu.
/// En mode `reference`, l'identifiant est gardé et le serveur redéployé. Permission :
/// `servers.content.manage`.
#[utoipa::path(post, path = "/api/admin/v1/servers/{id}/content/{slot}", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("slot" = String, Path)),
    request_body = InstallContentRequest,
    responses((status = 202, body = [ContentDto]), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn install(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path((id, slot)): Path<(Uuid, String)>,
    Json(request): Json<InstallContentRequest>,
) -> Result<(StatusCode, Json<Vec<ContentDto>>), ApiError> {
    session
        .require(&iam, Permission::ServersContentManage)
        .await?;
    let items = content::install(
        &state,
        &session.context,
        id,
        &slot,
        request.source.as_deref(),
        &request.version_id,
        &request.values,
    )
    .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(items.into_iter().map(|i| dto(i, None)).collect()),
    ))
}

/// Met à jour vers la version compatible la plus récente. Permission : `servers.content.manage`.
#[utoipa::path(post, path = "/api/admin/v1/servers/{id}/content/{slot}/{content_id}/update", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("slot" = String, Path), ("content_id" = Uuid, Path)),
    responses((status = 202, body = ContentDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn update(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path((id, slot, content_id)): Path<(Uuid, String, Uuid)>,
) -> Result<(StatusCode, Json<ContentDto>), ApiError> {
    session
        .require(&iam, Permission::ServersContentManage)
        .await?;
    let item = content::update(&state, &session.context, id, &slot, content_id).await?;
    Ok((StatusCode::ACCEPTED, Json(dto(item, None))))
}

/// Active ou désactive un contenu. Permission : `servers.content.manage`.
#[utoipa::path(patch, path = "/api/admin/v1/servers/{id}/content/{slot}/{content_id}", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("slot" = String, Path), ("content_id" = Uuid, Path)),
    request_body = ToggleContentRequest,
    responses((status = 200, body = ContentDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 502, body = Problem)))]
pub async fn toggle(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path((id, slot, content_id)): Path<(Uuid, String, Uuid)>,
    Json(request): Json<ToggleContentRequest>,
) -> Result<Json<ContentDto>, ApiError> {
    session
        .require(&iam, Permission::ServersContentManage)
        .await?;
    let item = content::toggle(
        &state,
        &session.context,
        id,
        &slot,
        content_id,
        request.enabled,
    )
    .await?;
    Ok(Json(dto(item, None)))
}

/// Modifie les valeurs saisies d'un contenu ; le serveur est redéployé pour que ses fichiers de
/// configuration les reprennent. Permission : `servers.content.manage`.
#[utoipa::path(put, path = "/api/admin/v1/servers/{id}/content/{slot}/{content_id}/values", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("slot" = String, Path), ("content_id" = Uuid, Path)),
    request_body = ContentValuesRequest,
    responses((status = 200, body = ContentDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn set_values(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path((id, slot, content_id)): Path<(Uuid, String, Uuid)>,
    Json(request): Json<ContentValuesRequest>,
) -> Result<Json<ContentDto>, ApiError> {
    session
        .require(&iam, Permission::ServersContentManage)
        .await?;
    let item = content::set_values(
        &state,
        &session.context,
        id,
        &slot,
        content_id,
        &request.values,
    )
    .await?;
    Ok(Json(dto(item, None)))
}

/// Supprime un contenu (et, sur demande, les dépendances devenues inutiles). Refusé s'il est
/// encore exigé par un autre. Permission : `servers.content.manage`.
#[utoipa::path(delete, path = "/api/admin/v1/servers/{id}/content/{slot}/{content_id}", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("slot" = String, Path), ("content_id" = Uuid, Path), RemoveParams),
    responses((status = 200, body = RemovedContentDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 502, body = Problem)))]
pub async fn remove(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path((id, slot, content_id)): Path<(Uuid, String, Uuid)>,
    Query(params): Query<RemoveParams>,
) -> Result<Json<RemovedContentDto>, ApiError> {
    session
        .require(&iam, Permission::ServersContentManage)
        .await?;
    let removed = content::remove(
        &state,
        &session.context,
        id,
        &slot,
        content_id,
        params.with_orphans.unwrap_or(false),
    )
    .await?;
    Ok(Json(RemovedContentDto { removed }))
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct UploadParams {
    /// Nom du fichier dans l'emplacement (`mymod.jar`).
    pub file_name: String,
}

/// Dépose un fichier envoyé (corps brut, `Content-Length` obligatoire, 256 Mio au plus) dans
/// l'emplacement, avec la source `upload`. Permission : `servers.content.manage`.
#[utoipa::path(post, path = "/api/admin/v1/servers/{id}/content/{slot}/upload", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("slot" = String, Path), UploadParams),
    request_body(content = Vec<u8>, content_type = "application/octet-stream"),
    responses((status = 201, body = ContentDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 411, body = Problem), (status = 422, body = Problem),
        (status = 502, body = Problem)))]
pub async fn upload(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path((id, slot)): Path<(Uuid, String)>,
    Query(params): Query<UploadParams>,
    headers: axum::http::HeaderMap,
    body: axum::body::Body,
) -> Result<(StatusCode, Json<ContentDto>), ApiError> {
    session
        .require(&iam, Permission::ServersContentManage)
        .await?;
    let size = super::files::content_length(&headers)?;
    let item = content::upload(
        &state,
        &session.context,
        id,
        &slot,
        &params.file_name,
        size,
        body.into_data_stream(),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(dto(item, None))))
}

/// Modpacks Modrinth, toutes versions du jeu (le modpack impose la sienne).
/// Permission : `servers.content.read`.
#[utoipa::path(get, path = "/api/admin/v1/servers/{id}/content/{slot}/modpacks/search", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("slot" = String, Path), SearchParams),
    responses((status = 200, body = ContentSearchDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn modpack_search(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path((id, slot)): Path<(Uuid, String)>,
    Query(params): Query<SearchParams>,
) -> Result<Json<ContentSearchDto>, ApiError> {
    session
        .require(&iam, Permission::ServersContentRead)
        .await?;
    let page = content::modpack_search(
        &state,
        id,
        &slot,
        params.q.as_deref().unwrap_or(""),
        params.offset.unwrap_or(0),
    )
    .await?;
    Ok(Json(ContentSearchDto {
        total: page.total,
        hits: page.hits.into_iter().map(Into::into).collect(),
    }))
}

/// Versions d'un modpack. Permission : `servers.content.read`.
#[utoipa::path(get, path = "/api/admin/v1/servers/{id}/content/{slot}/modpacks/{project}/versions", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("slot" = String, Path), ("project" = String, Path)),
    responses((status = 200, body = [ContentVersionDto]), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn modpack_versions(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path((id, slot, project)): Path<(Uuid, String, String)>,
) -> Result<Json<Vec<ContentVersionDto>>, ApiError> {
    session
        .require(&iam, Permission::ServersContentRead)
        .await?;
    let versions = content::modpack_versions(&state, id, &slot, &project).await?;
    Ok(Json(versions.into_iter().map(Into::into).collect()))
}

/// Installe un modpack : version du jeu et loader réglés selon lui (redéploiement s'ils
/// changent), fichiers côté serveur téléchargés et vérifiés, `overrides` extraits. Un seul
/// modpack par emplacement. Permission : `servers.content.manage`.
#[utoipa::path(post, path = "/api/admin/v1/servers/{id}/content/{slot}/modpacks", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("slot" = String, Path)),
    request_body = InstallContentRequest,
    responses((status = 202, body = [ContentDto]), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn install_modpack(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path((id, slot)): Path<(Uuid, String)>,
    Json(request): Json<InstallContentRequest>,
) -> Result<(StatusCode, Json<Vec<ContentDto>>), ApiError> {
    session
        .require(&iam, Permission::ServersContentManage)
        .await?;
    let items =
        content::install_modpack(&state, &session.context, id, &slot, &request.version_id).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(items.into_iter().map(|i| dto(i, None)).collect()),
    ))
}
