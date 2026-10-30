//! Provider Steam Workshop (ADR-0030) : API Web de Steam, clé de l'installation réglée dans le
//! panel et passée en en-tête (`x-webapi-key`), jamais dans l'adresse. Le jeu se désigne par son
//! identifiant d'application Steam (`108600` pour Project Zomboid). Un objet Workshop n'a pas de
//! versions : sa dernière publication en tient lieu, et le serveur du jeu le télécharge lui-même
//! (mode `reference`). Ses objets requis sont ses dépendances.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock};

use futures_util::future::BoxFuture;
use hyper::header::HeaderName;
use hyper::{Method, StatusCode, Uri};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::client::{Header, JsonClient, encode};
use super::provider::{
    Compat, ContentProvider, Dependency, InstalledRef, Project, ProjectHit, ProviderError,
    ProviderVersion, SearchPage, SearchQuery, SearchSort,
};

pub const DEFAULT_BASE_URL: &str = "https://api.steampowered.com";
/// Steam ne pagine pas au-delà de 1000 pages.
const MAX_PAGE: u32 = 1000;

#[derive(Debug, Clone)]
pub struct SteamWorkshop {
    base_url: String,
    client: JsonClient,
    key: Arc<RwLock<Option<String>>>,
}

impl SteamWorkshop {
    pub fn new(base_url: &str) -> Result<Self, ProviderError> {
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            client: JsonClient::new("steam", base_url)?,
            key: Arc::new(RwLock::new(None)),
        })
    }

    fn header(key: String) -> Header {
        (HeaderName::from_static("x-webapi-key"), key)
    }

    async fn get<T: DeserializeOwned>(&self, path_and_query: &str) -> Result<T, ProviderError> {
        let key = self
            .key
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .ok_or(ProviderError::NotConfigured)?;
        let value = self
            .client
            .call(Method::GET, path_and_query, None, Some(&Self::header(key)))
            .await?;
        serde_json::from_value(value).map_err(|e| ProviderError::Unavailable(e.to_string()))
    }

    /// Fiche d'un objet : description sans balises, objets requis.
    async fn details(&self, id: u64) -> Result<RawFile, ProviderError> {
        let raw: Response<Details> = self
            .get(&format!(
                "/IPublishedFileService/GetDetails/v1/?publishedfileids%5B0%5D={id}&includechildren=true&strip_description_bbcode=true"
            ))
            .await?;
        raw.response
            .publishedfiledetails
            .into_iter()
            .find(|file| file.result == 1)
            .ok_or(ProviderError::NotFound)
    }
}

/// Type de requête de `QueryFiles` (`EPublishedFileQueryType`) : par pertinence quand un texte
/// est cherché, sinon par abonnés.
fn query_type(sort: SearchSort, text: &str) -> u8 {
    match sort {
        SearchSort::Relevance if !text.is_empty() => 12,
        SearchSort::Relevance | SearchSort::Downloads => 9,
        SearchSort::Updated => 21,
        SearchSort::Newest => 1,
    }
}

/// Recherche : jeu, texte, tri ; Steam pagine par numéro de page.
fn search_path(query: &SearchQuery, appid: u64) -> String {
    let limit = query.limit.clamp(1, 50);
    let page = (query.offset / limit + 1).min(MAX_PAGE);
    format!(
        "/IPublishedFileService/QueryFiles/v1/?query_type={}&page={page}&numperpage={limit}&appid={appid}&search_text={}&return_short_description=true",
        query_type(query.sort, &query.text),
        encode(&query.text),
    )
}

/// Identifiant numérique Steam : jamais de caractère à échapper.
fn checked_id(id: &str) -> Result<u64, ProviderError> {
    id.parse().map_err(|_| ProviderError::NotFound)
}

/// Page Workshop d'un objet.
pub fn page_url(id: &str) -> String {
    format!("https://steamcommunity.com/sharedfiles/filedetails/?id={id}")
}

#[derive(Deserialize)]
struct Response<T> {
    response: T,
}

#[derive(Deserialize)]
struct Query {
    #[serde(default)]
    total: u64,
    #[serde(default)]
    publishedfiledetails: Vec<RawFile>,
}

#[derive(Deserialize)]
struct Details {
    #[serde(default)]
    publishedfiledetails: Vec<RawFile>,
}

#[derive(Deserialize)]
struct RawChild {
    publishedfileid: String,
}

#[derive(Deserialize)]
struct RawFile {
    #[serde(default = "found")]
    result: u8,
    publishedfileid: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    short_description: String,
    #[serde(default)]
    file_description: String,
    #[serde(default)]
    preview_url: Option<String>,
    #[serde(default, deserialize_with = "number")]
    subscriptions: u64,
    #[serde(default, deserialize_with = "number")]
    consumer_appid: u64,
    #[serde(default, deserialize_with = "number")]
    time_updated: u64,
    #[serde(default)]
    children: Vec<RawChild>,
}

/// Les réponses de recherche n'ont pas toutes de `result` : un objet listé existe.
fn found() -> u8 {
    1
}

/// Nombre que Steam donne tantôt en nombre, tantôt en chaîne.
fn number<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Number(u64),
        Text(String),
    }
    Ok(match Raw::deserialize(deserializer)? {
        Raw::Number(n) => n,
        Raw::Text(text) => text.parse().unwrap_or(0),
    })
}

impl RawFile {
    fn hit(self) -> ProjectHit {
        ProjectHit {
            page_url: Some(page_url(&self.publishedfileid)),
            download_allowed: true,
            slug: self.publishedfileid.clone(),
            project_id: self.publishedfileid,
            title: self.title,
            description: self.short_description,
            // L'auteur n'est qu'un identifiant Steam : rien de lisible à montrer.
            author: String::new(),
            icon_url: self.preview_url.filter(|url| url.starts_with("https://")),
            downloads: self.subscriptions,
        }
    }

    /// La dernière publication, seule « version » d'un objet Workshop.
    fn version(&self) -> ProviderVersion {
        let updated = i64::try_from(self.time_updated)
            .ok()
            .and_then(|seconds| OffsetDateTime::from_unix_timestamp(seconds).ok());
        ProviderVersion {
            id: self.publishedfileid.clone(),
            project_id: self.publishedfileid.clone(),
            name: self.title.clone(),
            version_number: updated.map(|at| at.date().to_string()).unwrap_or_default(),
            published: updated
                .and_then(|at| at.format(&Rfc3339).ok())
                .unwrap_or_default(),
            loaders: Vec::new(),
            game_versions: Vec::new(),
            // Le serveur du jeu télécharge lui-même : Kuro ne dépose aucun fichier.
            files: Vec::new(),
            dependencies: self
                .children
                .iter()
                .map(|child| Dependency {
                    project_id: Some(child.publishedfileid.clone()),
                    version_id: Some(child.publishedfileid.clone()),
                    required: true,
                })
                .collect(),
        }
    }
}

impl ContentProvider for SteamWorkshop {
    fn search(&self, query: &SearchQuery) -> BoxFuture<'_, Result<SearchPage, ProviderError>> {
        let query = query.clone();
        Box::pin(async move {
            let appid = query
                .compat
                .game
                .as_deref()
                .ok_or(ProviderError::NotFound)
                .and_then(checked_id)?;
            let raw: Response<Query> = self.get(&search_path(&query, appid)).await?;
            Ok(SearchPage {
                total: raw.response.total,
                hits: raw
                    .response
                    .publishedfiledetails
                    .into_iter()
                    .map(RawFile::hit)
                    .collect(),
            })
        })
    }

    fn project(&self, project_id: &str) -> BoxFuture<'_, Result<Project, ProviderError>> {
        let project_id = project_id.to_owned();
        Box::pin(async move {
            let file = self.details(checked_id(&project_id)?).await?;
            Ok(Project {
                id: file.publishedfileid,
                title: file.title,
                icon_url: file.preview_url.filter(|url| url.starts_with("https://")),
                description: Some(file.file_description),
                download_allowed: true,
            })
        })
    }

    /// La seule « version », si l'objet appartient bien au jeu du serveur.
    fn versions(
        &self,
        project_id: &str,
        compat: &Compat,
    ) -> BoxFuture<'_, Result<Vec<ProviderVersion>, ProviderError>> {
        let (project_id, compat) = (project_id.to_owned(), compat.clone());
        Box::pin(async move {
            let file = self.details(checked_id(&project_id)?).await?;
            let game = compat.game.as_deref().and_then(|g| g.parse::<u64>().ok());
            if game.is_some_and(|appid| appid != file.consumer_appid) {
                return Ok(Vec::new());
            }
            Ok(vec![file.version()])
        })
    }

    fn version(&self, version_id: &str) -> BoxFuture<'_, Result<ProviderVersion, ProviderError>> {
        let version_id = version_id.to_owned();
        Box::pin(async move { Ok(self.details(checked_id(&version_id)?).await?.version()) })
    }

    /// Le serveur du jeu met lui-même ses objets à jour.
    fn latest(
        &self,
        _installed: &[InstalledRef],
        _compat: &Compat,
    ) -> BoxFuture<'_, Result<HashMap<String, ProviderVersion>, ProviderError>> {
        Box::pin(async { Ok(HashMap::new()) })
    }

    fn latest_game_version(&self) -> BoxFuture<'_, Result<String, ProviderError>> {
        Box::pin(async { Err(ProviderError::NotFound) })
    }

    /// Kuro ne télécharge rien depuis le Workshop.
    fn is_allowed_download(&self, _url: &str) -> bool {
        false
    }

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
        self.client.clear_cache();
    }

    /// Une recherche réduite à son total, hors cache, avec la clé à vérifier.
    fn verify_key(&self, key: &str) -> BoxFuture<'_, Result<(), ProviderError>> {
        let header = Self::header(key.to_owned());
        Box::pin(async move {
            let uri: Uri = format!(
                "{}/IPublishedFileService/QueryFiles/v1/?query_type=1&page=1&numperpage=1&appid=440&totalonly=true",
                self.base_url
            )
            .parse()
            .map_err(|_| ProviderError::Unavailable("invalid steam url".to_owned()))?;
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
                    "steam answered {other}"
                ))),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(sort: SearchSort, text: &str, offset: u32) -> SearchQuery {
        SearchQuery {
            text: text.to_owned(),
            compat: Compat {
                project_type: "mod".to_owned(),
                game: Some("108600".to_owned()),
                class: None,
                loader: None,
                game_version: String::new(),
            },
            offset,
            limit: 20,
            sort,
            categories: Vec::new(),
        }
    }

    #[test]
    fn the_search_maps_sorts_and_pages() {
        let text = search_path(&query(SearchSort::Relevance, "hydro craft", 40), 108_600);
        assert!(
            text.contains("query_type=12&page=3&numperpage=20&appid=108600"),
            "{text}"
        );
        assert!(text.contains("search_text=hydro%20craft"), "{text}");
        let popular = search_path(&query(SearchSort::Relevance, "", 0), 108_600);
        assert!(popular.contains("query_type=9&page=1"), "{popular}");
        assert!(search_path(&query(SearchSort::Updated, "", 0), 1).contains("query_type=21&"));
        assert!(search_path(&query(SearchSort::Newest, "", 0), 1).contains("query_type=1&"));
        // La clé ne passe jamais dans l'adresse.
        assert!(!text.contains("key"), "{text}");
    }

    #[test]
    fn an_item_is_its_only_version_with_its_required_items() {
        let file: RawFile = serde_json::from_value(serde_json::json!({
            "result": 1,
            "publishedfileid": "2392709985",
            "title": "Hydrocraft",
            "file_description": "Mod ID: Hydrocraft",
            "subscriptions": "123456",
            "consumer_appid": 108600,
            "time_updated": 1_700_000_000,
            "children": [{ "publishedfileid": "498441420", "file_type": 0 }],
        }))
        .unwrap();
        let version = file.version();
        assert_eq!(version.id, "2392709985");
        assert_eq!(version.version_number, "2023-11-14");
        assert!(version.files.is_empty());
        assert_eq!(version.dependencies.len(), 1);
        assert_eq!(
            version.dependencies[0].version_id.as_deref(),
            Some("498441420")
        );
        assert_eq!(file.hit().downloads, 123_456);
    }
}
