//! Provider CurseForge (ADR-0029) : API REST v1, clé d'API de l'installation (`x-api-key`),
//! réglée dans le panel. Le jeu et la classe de contenu déclarés par le prefab (`minecraft`,
//! `mods`) sont résolus par leur nom chez CurseForge. Les fichiers sont vérifiés par SHA-1, la
//! seule empreinte publiée ; un mod dont l'auteur interdit la distribution par des tiers n'a pas
//! d'adresse de téléchargement et n'est jamais téléchargé par Kuro.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock};

use futures_util::future::BoxFuture;
use hyper::header::HeaderName;
use hyper::{Method, StatusCode, Uri};
use kuro_protocol::api::ContentHash;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::client::{Header, JsonClient, encode};
use super::provider::{
    Category, Compat, ContentProvider, Dependency, InstalledRef, Project, ProjectHit,
    ProviderError, ProviderFile, ProviderVersion, SearchPage, SearchQuery, SearchSort,
    accepted_loaders,
};

pub const DEFAULT_BASE_URL: &str = "https://api.curseforge.com";
/// Hôtes des fichiers CurseForge.
const DOWNLOAD_HOSTS: [&str; 2] = ["edge.forgecdn.net", "mediafilez.forgecdn.net"];
/// Parcours de la liste des jeux : 50 par page.
const MAX_GAME_PAGES: u32 = 20;
/// La recherche s'arrête à 10 000 résultats (index + taille de page).
const MAX_RESULTS: u32 = 10_000;
/// `FileRelationType::RequiredDependency`.
const REQUIRED_DEPENDENCY: u8 = 3;
/// `HashAlgo::Sha1`.
const SHA1: u8 = 1;
/// Catégories d'une recherche au plus (limite de CurseForge).
const MAX_CATEGORIES: usize = 10;

/// Champ de tri (`ModsSearchSortField`) : la pertinence de CurseForge est sa popularité.
fn sort_field(sort: SearchSort) -> u8 {
    match sort {
        SearchSort::Relevance => 2,
        SearchSort::Updated => 3,
        SearchSort::Downloads => 6,
        SearchSort::Newest => 11,
    }
}

#[derive(Debug, Clone)]
pub struct CurseForge {
    base_url: String,
    client: JsonClient,
    key: Arc<RwLock<Option<String>>>,
}

impl CurseForge {
    pub fn new(base_url: &str) -> Result<Self, ProviderError> {
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            client: JsonClient::new("curseforge", base_url)?,
            key: Arc::new(RwLock::new(None)),
        })
    }

    fn header(&self) -> Result<Header, ProviderError> {
        let key = self
            .key
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .ok_or(ProviderError::NotConfigured)?;
        Ok((HeaderName::from_static("x-api-key"), key))
    }

    async fn call<T: DeserializeOwned>(
        &self,
        method: Method,
        path_and_query: &str,
        body: Option<Value>,
    ) -> Result<T, ProviderError> {
        let header = self.header()?;
        let value = self
            .client
            .call(method, path_and_query, body, Some(&header))
            .await?;
        serde_json::from_value(value).map_err(|e| ProviderError::Unavailable(e.to_string()))
    }

    async fn get<T: DeserializeOwned>(&self, path_and_query: &str) -> Result<T, ProviderError> {
        self.call(Method::GET, path_and_query, None).await
    }

    /// Identifiant CurseForge du jeu déclaré par le prefab (`minecraft`).
    async fn game_id(&self, slug: &str) -> Result<u64, ProviderError> {
        for page in 0..MAX_GAME_PAGES {
            let games: Paged<RawGame> = self
                .get(&format!("/v1/games?index={}&pageSize=50", page * 50))
                .await?;
            if let Some(game) = games.data.iter().find(|g| g.slug == slug) {
                return Ok(game.id);
            }
            if u64::from((page + 1) * 50) >= games.pagination.total_count {
                break;
            }
        }
        Err(ProviderError::NotFound)
    }

    /// Classe de contenu du jeu (`mods`) : par son slug ou son nom, `mc-mods` pour Minecraft.
    async fn class_id(&self, game_id: u64, class: &str) -> Result<u64, ProviderError> {
        let classes: Data<Vec<RawCategory>> = self
            .get(&format!("/v1/categories?gameId={game_id}&classesOnly=true"))
            .await?;
        classes
            .data
            .iter()
            .find(|c| {
                c.slug == class
                    || c.name.eq_ignore_ascii_case(class)
                    || c.slug == format!("mc-{class}")
            })
            .map(|c| c.id)
            .ok_or(ProviderError::NotFound)
    }

    /// Jeu et classe de la compatibilité, résolus chez CurseForge.
    async fn scope(&self, compat: &Compat) -> Result<(u64, Option<u64>), ProviderError> {
        let game = compat.game.as_deref().ok_or(ProviderError::NotFound)?;
        let game_id = self.game_id(game).await?;
        let class_id = match compat.class.as_deref() {
            Some(class) => Some(self.class_id(game_id, class).await?),
            None => None,
        };
        Ok((game_id, class_id))
    }

    async fn files(&self, file_ids: &[u64]) -> Result<Vec<RawFile>, ProviderError> {
        if file_ids.is_empty() {
            return Ok(Vec::new());
        }
        let files: Data<Vec<RawFile>> = self
            .call(
                Method::POST,
                "/v1/mods/files",
                Some(json!({ "fileIds": file_ids })),
            )
            .await?;
        Ok(files.data)
    }
}

/// Codes `ModLoaderType` des loaders qu'un serveur accepte (Quilt accepte Fabric).
fn loader_types(loader: &str) -> Vec<u8> {
    accepted_loaders(loader)
        .into_iter()
        .filter_map(|l| match l {
            "forge" => Some(1),
            "fabric" => Some(4),
            "quilt" => Some(5),
            "neoforge" => Some(6),
            _ => None,
        })
        .collect()
}

/// Loaders nommés dans les versions de jeu d'un fichier (`Fabric`, `NeoForge`…).
const LOADER_NAMES: [&str; 4] = ["forge", "fabric", "quilt", "neoforge"];

/// Identifiant numérique de CurseForge : jamais de caractère à échapper.
fn checked_id(id: &str) -> Result<u64, ProviderError> {
    id.parse().map_err(|_| ProviderError::NotFound)
}

/// Requête de recherche : jeu, classe, texte, tri, catégories choisies (toutes), et
/// loaders acceptés avec la version du jeu (CurseForge n'accepte le loader qu'avec elle).
fn search_path(query: &SearchQuery, game_id: u64, class_id: Option<u64>) -> String {
    let index = query.offset.min(MAX_RESULTS.saturating_sub(query.limit));
    let mut path = format!(
        "/v1/mods/search?gameId={game_id}&searchFilter={}&sortField={}&sortOrder=desc&index={index}&pageSize={}",
        encode(&query.text),
        sort_field(query.sort),
        query.limit.min(50),
    );
    if let Some(class_id) = class_id {
        path.push_str(&format!("&classId={class_id}"));
    }
    // Catégories choisies : CurseForge exige chacune d'elles.
    let categories: Vec<u64> = query
        .categories
        .iter()
        .filter_map(|c| c.parse().ok())
        .take(MAX_CATEGORIES)
        .collect();
    match categories.as_slice() {
        [] => {}
        [single] => path.push_str(&format!("&categoryId={single}")),
        several => {
            let list: Vec<String> = several.iter().map(u64::to_string).collect();
            path.push_str(&format!(
                "&categoryIds={}",
                encode(&format!("[{}]", list.join(",")))
            ));
        }
    }
    // Le loader n'est accepté qu'avec une version du jeu.
    if !query.compat.game_version.is_empty() {
        path.push_str(&format!(
            "&gameVersion={}",
            encode(&query.compat.game_version)
        ));
        match query.compat.loader.as_deref().map(loader_types).as_deref() {
            Some([single]) => path.push_str(&format!("&modLoaderType={single}")),
            Some(several) if !several.is_empty() => {
                let list: Vec<String> = several.iter().map(u8::to_string).collect();
                path.push_str(&format!(
                    "&modLoaderTypes={}",
                    encode(&format!("[{}]", list.join(",")))
                ));
            }
            _ => {}
        }
    }
    path
}

#[derive(Deserialize)]
struct Data<T> {
    data: T,
}

#[derive(Deserialize)]
struct Paged<T> {
    data: Vec<T>,
    pagination: Pagination,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pagination {
    total_count: u64,
}

#[derive(Deserialize)]
struct RawGame {
    id: u64,
    slug: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCategory {
    id: u64,
    slug: String,
    name: String,
    #[serde(default)]
    class_id: Option<u64>,
    #[serde(default)]
    is_class: Option<bool>,
    #[serde(default)]
    icon_url: Option<String>,
}

#[derive(Deserialize)]
struct RawAuthor {
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawLogo {
    #[serde(default)]
    thumbnail_url: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawLinks {
    #[serde(default)]
    website_url: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawFileIndex {
    game_version: String,
    file_id: u64,
    #[serde(default)]
    mod_loader: Option<u8>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMod {
    id: u64,
    slug: String,
    name: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    download_count: f64,
    #[serde(default)]
    authors: Vec<RawAuthor>,
    #[serde(default)]
    logo: Option<RawLogo>,
    #[serde(default)]
    links: Option<RawLinks>,
    #[serde(default)]
    allow_mod_distribution: Option<bool>,
    #[serde(default)]
    latest_files_indexes: Vec<RawFileIndex>,
    #[serde(default)]
    latest_files: Vec<RawFile>,
}

impl RawMod {
    /// Seul un refus explicite de l'auteur bloque le téléchargement.
    fn download_allowed(&self) -> bool {
        self.allow_mod_distribution != Some(false)
    }

    fn icon_url(&self) -> Option<String> {
        self.logo.as_ref().and_then(|l| l.thumbnail_url.clone())
    }
}

#[derive(Deserialize)]
struct RawHash {
    value: String,
    algo: u8,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawDependency {
    mod_id: u64,
    relation_type: u8,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawFile {
    id: u64,
    mod_id: u64,
    display_name: String,
    file_name: String,
    file_date: String,
    #[serde(default)]
    file_length: u64,
    #[serde(default)]
    download_url: Option<String>,
    #[serde(default)]
    hashes: Vec<RawHash>,
    #[serde(default)]
    game_versions: Vec<String>,
    #[serde(default)]
    dependencies: Vec<RawDependency>,
    #[serde(default)]
    is_available: Option<bool>,
    #[serde(default)]
    is_server_pack: Option<bool>,
}

impl RawFile {
    /// Fichier proposé : disponible, et pas l'archive de serveur d'un modpack.
    fn usable(&self) -> bool {
        self.is_available != Some(false) && self.is_server_pack != Some(true)
    }
}

/// Adresse de téléchargement : les noms de fichiers CurseForge contiennent parfois des espaces.
fn download_url(url: &str) -> String {
    url.replace(' ', "%20")
}

impl From<RawFile> for ProviderVersion {
    fn from(raw: RawFile) -> Self {
        let (loaders, game_versions): (Vec<String>, Vec<String>) = raw
            .game_versions
            .iter()
            .map(|v| v.to_ascii_lowercase())
            .partition(|v| LOADER_NAMES.contains(&v.as_str()));
        let sha1 = raw
            .hashes
            .iter()
            .find(|h| h.algo == SHA1)
            .map(|h| h.value.to_ascii_lowercase());
        // Sans adresse (distribution refusée) ou sans SHA-1 : rien à télécharger.
        let files = match (raw.download_url.as_deref(), sha1) {
            (Some(url), Some(sha1)) => vec![ProviderFile {
                url: download_url(url),
                file_name: raw.file_name,
                hash: ContentHash::Sha1(sha1),
                size: raw.file_length,
                primary: true,
            }],
            _ => Vec::new(),
        };
        Self {
            id: raw.id.to_string(),
            project_id: raw.mod_id.to_string(),
            name: raw.display_name.clone(),
            version_number: raw.display_name,
            published: raw.file_date,
            loaders,
            // Les versions du jeu gardent leur casse d'origine (`1.21.4`).
            game_versions: raw
                .game_versions
                .into_iter()
                .filter(|v| game_versions.contains(&v.to_ascii_lowercase()))
                .collect(),
            files,
            dependencies: raw
                .dependencies
                .into_iter()
                .filter(|d| d.relation_type == REQUIRED_DEPENDENCY)
                .map(|d| Dependency {
                    project_id: Some(d.mod_id.to_string()),
                    version_id: None,
                    required: true,
                })
                .collect(),
        }
    }
}

impl ContentProvider for CurseForge {
    fn search(&self, query: &SearchQuery) -> BoxFuture<'_, Result<SearchPage, ProviderError>> {
        let query = query.clone();
        Box::pin(async move {
            let (game_id, class_id) = self.scope(&query.compat).await?;
            let path = search_path(&query, game_id, class_id);
            let raw: Paged<RawMod> = self.get(&path).await?;
            Ok(SearchPage {
                total: raw.pagination.total_count.min(u64::from(MAX_RESULTS)),
                hits: raw
                    .data
                    .into_iter()
                    .map(|m| ProjectHit {
                        download_allowed: m.download_allowed(),
                        icon_url: m.icon_url(),
                        page_url: m.links.and_then(|l| l.website_url),
                        project_id: m.id.to_string(),
                        slug: m.slug,
                        title: m.name,
                        description: m.summary,
                        author: m
                            .authors
                            .into_iter()
                            .next()
                            .map(|a| a.name)
                            .unwrap_or_default(),
                        // Nombre entier dans les faits ; la valeur reste bornée.
                        downloads: m.download_count.max(0.0).min(u64::MAX as f64) as u64,
                    })
                    .collect(),
            })
        })
    }

    fn project(&self, project_id: &str) -> BoxFuture<'_, Result<Project, ProviderError>> {
        let project_id = project_id.to_owned();
        Box::pin(async move {
            let raw: Data<RawMod> = self
                .get(&format!("/v1/mods/{}", checked_id(&project_id)?))
                .await?;
            Ok(Project {
                download_allowed: raw.data.download_allowed(),
                icon_url: raw.data.icon_url(),
                description: Some(raw.data.summary.clone()),
                id: raw.data.id.to_string(),
                title: raw.data.name,
            })
        })
    }

    fn versions(
        &self,
        project_id: &str,
        compat: &Compat,
    ) -> BoxFuture<'_, Result<Vec<ProviderVersion>, ProviderError>> {
        let (project_id, compat) = (project_id.to_owned(), compat.clone());
        Box::pin(async move {
            let mut path = format!(
                "/v1/mods/{}/files?index=0&pageSize=50",
                checked_id(&project_id)?
            );
            if !compat.game_version.is_empty() {
                path.push_str(&format!("&gameVersion={}", encode(&compat.game_version)));
            }
            let raw: Paged<RawFile> = self.get(&path).await?;
            let mut versions: Vec<ProviderVersion> = raw
                .data
                .into_iter()
                .filter(RawFile::usable)
                .map(ProviderVersion::from)
                .filter(|v| v.is_compatible(&compat))
                .collect();
            versions.sort_by(|a, b| b.published.cmp(&a.published));
            Ok(versions)
        })
    }

    fn version(&self, version_id: &str) -> BoxFuture<'_, Result<ProviderVersion, ProviderError>> {
        let version_id = version_id.to_owned();
        Box::pin(async move {
            self.files(&[checked_id(&version_id)?])
                .await?
                .into_iter()
                .find(RawFile::usable)
                .map(ProviderVersion::from)
                .ok_or(ProviderError::NotFound)
        })
    }

    /// Par projet : l'index des derniers fichiers de chaque mod, par version du jeu et loader.
    fn latest(
        &self,
        installed: &[InstalledRef],
        compat: &Compat,
    ) -> BoxFuture<'_, Result<HashMap<String, ProviderVersion>, ProviderError>> {
        let ids: Vec<u64> = installed
            .iter()
            .filter_map(|i| i.project_id.parse().ok())
            .collect();
        let compat = compat.clone();
        Box::pin(async move {
            if ids.is_empty() {
                return Ok(HashMap::new());
            }
            let mods: Data<Vec<RawMod>> = self
                .call(Method::POST, "/v1/mods", Some(json!({ "modIds": ids })))
                .await?;
            let types = compat.loader.as_deref().map(loader_types);
            let mut wanted = Vec::new();
            let mut fallback = Vec::new();
            for m in mods.data {
                let indexed = m
                    .latest_files_indexes
                    .iter()
                    .filter(|i| {
                        (compat.game_version.is_empty() || i.game_version == compat.game_version)
                            && types.as_ref().is_none_or(|types| {
                                i.mod_loader.is_some_and(|l| types.contains(&l))
                            })
                    })
                    .map(|i| i.file_id)
                    .max();
                match indexed {
                    Some(file_id) => wanted.push(file_id),
                    // Sans index (jeu sans versions, comme ARK) : les derniers fichiers du mod.
                    None => fallback.extend(m.latest_files.into_iter().filter(RawFile::usable)),
                }
            }
            let mut latest: HashMap<String, ProviderVersion> = HashMap::new();
            let files = self.files(&wanted).await?.into_iter().chain(fallback);
            for version in files.filter(RawFile::usable).map(ProviderVersion::from) {
                if !version.is_compatible(&compat) {
                    continue;
                }
                let newer = latest
                    .get(&version.project_id)
                    .is_none_or(|current| version.published > current.published);
                if newer {
                    latest.insert(version.project_id.clone(), version);
                }
            }
            Ok(latest)
        })
    }

    /// Catégories de la classe de contenu (`mods`) du jeu, hors classes elles-mêmes.
    fn categories(&self, compat: &Compat) -> BoxFuture<'_, Result<Vec<Category>, ProviderError>> {
        let compat = compat.clone();
        Box::pin(async move {
            let (game_id, class_id) = self.scope(&compat).await?;
            let mut path = format!("/v1/categories?gameId={game_id}");
            if let Some(class_id) = class_id {
                path.push_str(&format!("&classId={class_id}"));
            }
            let raw: Data<Vec<RawCategory>> = self.get(&path).await?;
            let mut categories: Vec<Category> = raw
                .data
                .into_iter()
                .filter(|c| c.is_class != Some(true))
                .filter(|c| class_id.is_none() || c.class_id == class_id)
                .map(|c| Category {
                    id: c.id.to_string(),
                    name: c.name,
                    icon_svg: None,
                    icon_url: c.icon_url,
                })
                .collect();
            categories.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(categories)
        })
    }

    /// La version du jeu à figer vient de Modrinth.
    fn latest_game_version(&self) -> BoxFuture<'_, Result<String, ProviderError>> {
        Box::pin(async { Err(ProviderError::NotFound) })
    }

    fn is_allowed_download(&self, url: &str) -> bool {
        url.parse::<Uri>().is_ok_and(|uri| {
            uri.scheme_str() == Some("https")
                && uri.port().is_none()
                && uri
                    .host()
                    .is_some_and(|host| DOWNLOAD_HOSTS.contains(&host))
        })
    }

    /// Les modpacks viennent de Modrinth (`.mrpack`).
    fn is_allowed_pack_download(&self, _url: &str) -> bool {
        false
    }

    fn fetch(
        &self,
        _url: &str,
        _max_bytes: usize,
    ) -> BoxFuture<'_, Result<Vec<u8>, ProviderError>> {
        Box::pin(async { Err(ProviderError::NotFound) })
    }

    fn configured(&self) -> bool {
        self.key
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }

    fn set_key(&self, key: Option<String>) {
        *self.key.write().unwrap_or_else(PoisonError::into_inner) = key;
        // Réponses obtenues avec l'ancienne clé : oubliées.
        self.client.clear_cache();
    }

    /// Une requête légère avec la clé, hors cache : seule la réponse de CurseForge compte.
    fn verify_key(&self, key: &str) -> BoxFuture<'_, Result<(), ProviderError>> {
        let header: Header = (HeaderName::from_static("x-api-key"), key.to_owned());
        Box::pin(async move {
            let uri: Uri = format!("{}/v1/games?index=0&pageSize=1", self.base_url)
                .parse()
                .map_err(|_| ProviderError::Unavailable("invalid curseforge url".to_owned()))?;
            let (status, _) = self
                .client
                .request(Method::GET, &uri, None, Some(&header), 64 * 1024)
                .await?;
            match status {
                StatusCode::OK => Ok(()),
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                    Err(ProviderError::Unauthorized)
                }
                other => Err(ProviderError::Unavailable(format!(
                    "curseforge answered {other}"
                ))),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(versions: &[&str], download_url: Option<&str>) -> RawFile {
        RawFile {
            id: 42,
            mod_id: 7,
            display_name: "JEI 19.0.0".to_owned(),
            file_name: "jei 19.0.0.jar".to_owned(),
            file_date: "2026-10-01T00:00:00Z".to_owned(),
            file_length: 2048,
            download_url: download_url.map(str::to_owned),
            hashes: vec![
                RawHash {
                    value: "AB".repeat(16),
                    algo: 2,
                },
                RawHash {
                    value: "CD".repeat(20),
                    algo: SHA1,
                },
            ],
            game_versions: versions.iter().map(|v| (*v).to_owned()).collect(),
            dependencies: vec![
                RawDependency {
                    mod_id: 8,
                    relation_type: REQUIRED_DEPENDENCY,
                },
                RawDependency {
                    mod_id: 9,
                    relation_type: 2,
                },
            ],
            is_available: Some(true),
            is_server_pack: Some(false),
        }
    }

    #[test]
    fn a_file_splits_loaders_from_game_versions_and_keeps_its_sha1() {
        let version = ProviderVersion::from(file(
            &["1.21.4", "Fabric", "Quilt", "Client", "Server"],
            Some("https://edge.forgecdn.net/files/1/42/jei 19.0.0.jar"),
        ));
        assert_eq!(version.id, "42");
        assert_eq!(version.project_id, "7");
        assert_eq!(version.loaders, ["fabric", "quilt"]);
        assert_eq!(version.game_versions, ["1.21.4", "Client", "Server"]);
        let primary = version.primary_file().unwrap();
        assert_eq!(primary.hash, ContentHash::Sha1("cd".repeat(20)));
        assert_eq!(
            primary.url,
            "https://edge.forgecdn.net/files/1/42/jei%2019.0.0.jar"
        );
        assert_eq!(version.dependencies.len(), 1, "only required dependencies");
        assert_eq!(version.dependencies[0].project_id.as_deref(), Some("8"));
    }

    #[test]
    fn a_file_without_download_url_has_nothing_to_download() {
        let version = ProviderVersion::from(file(&["1.21.4", "Forge"], None));
        assert!(version.files.is_empty());
        assert!(version.primary_file().is_none());
    }

    #[test]
    fn a_quilt_server_asks_for_quilt_and_fabric_files() {
        assert_eq!(loader_types("quilt"), [5, 4]);
        assert_eq!(loader_types("neoforge"), [6]);
        assert!(loader_types("paper").is_empty());
    }

    fn query(sort: SearchSort, categories: &[&str]) -> SearchQuery {
        SearchQuery {
            text: "map".to_owned(),
            compat: Compat {
                project_type: "mod".to_owned(),
                game: Some("minecraft".to_owned()),
                class: Some("mods".to_owned()),
                loader: Some("quilt".to_owned()),
                game_version: "1.21.4".to_owned(),
            },
            offset: 0,
            limit: 20,
            sort,
            categories: categories.iter().map(|c| (*c).to_owned()).collect(),
        }
    }

    #[test]
    fn the_search_sorts_and_filters_by_category() {
        let several = search_path(&query(SearchSort::Downloads, &["421", "424"]), 432, Some(6));
        assert!(several.contains("gameId=432&"), "{several}");
        assert!(several.contains("&classId=6"), "{several}");
        assert!(several.contains("&sortField=6&"), "{several}");
        assert!(
            several.contains("&categoryIds=%5B421%2C424%5D"),
            "{several}"
        );
        assert!(several.contains("&modLoaderTypes=%5B5%2C4%5D"), "{several}");
        let single = search_path(&query(SearchSort::Newest, &["421"]), 432, Some(6));
        assert!(single.contains("&sortField=11&"), "{single}");
        assert!(single.contains("&categoryId=421"), "{single}");
        let none = search_path(
            &query(SearchSort::Relevance, &["not-a-number"]),
            432,
            Some(6),
        );
        assert!(none.contains("&sortField=2&"), "{none}");
        assert!(!none.contains("categoryId"), "{none}");
    }

    #[test]
    fn only_forgecdn_files_are_downloaded() {
        let source = CurseForge::new(DEFAULT_BASE_URL).unwrap();
        assert!(source.is_allowed_download("https://edge.forgecdn.net/files/1/42/jei.jar"));
        assert!(source.is_allowed_download("https://mediafilez.forgecdn.net/files/1/42/jei.jar"));
        assert!(!source.is_allowed_download("http://edge.forgecdn.net/files/1/42/jei.jar"));
        assert!(!source.is_allowed_download("https://edge.forgecdn.net:8443/x.jar"));
        assert!(!source.is_allowed_download("https://evil.example/x.jar"));
    }

    #[test]
    fn without_a_key_the_source_is_not_configured() {
        let source = CurseForge::new(DEFAULT_BASE_URL).unwrap();
        assert!(!source.configured());
        assert!(matches!(source.header(), Err(ProviderError::NotConfigured)));
        source.set_key(Some("key".to_owned()));
        assert!(source.configured());
        source.set_key(None);
        assert!(!source.configured());
    }
}
