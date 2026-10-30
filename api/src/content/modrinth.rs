//! Provider Modrinth (ADR-0020) : API v2 en HTTPS (rustls, autorités publiques), réponses de
//! 8 Mio au plus, mises en cache cinq minutes. Seuls les fichiers de `cdn.modrinth.com` sont
//! acceptés au téléchargement.

use std::collections::HashMap;
use std::time::Duration;

use futures_util::future::BoxFuture;
use hyper::{Method, StatusCode, Uri};
use kuro_protocol::api::ContentHash;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::time::timeout;

use super::client::{JsonClient, encode};
use super::provider::{
    Category, Compat, ContentProvider, Dependency, InstalledRef, Loader, Project, ProjectHit,
    ProviderError, ProviderFile, ProviderVersion, SearchPage, SearchQuery, SearchSort,
    accepted_loaders,
};

pub const DEFAULT_BASE_URL: &str = "https://api.modrinth.com/v2";
const DOWNLOAD_HOST: &str = "cdn.modrinth.com";
/// Hôtes autorisés par le format `.mrpack` pour les fichiers d'un modpack.
const PACK_HOSTS: [&str; 4] = [
    "cdn.modrinth.com",
    "github.com",
    "raw.githubusercontent.com",
    "gitlab.com",
];
/// Icône de loader plus grande : ignorée (celles de Modrinth font moins de 2 Kio).
const MAX_LOADER_ICON_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone)]
pub struct Modrinth {
    client: JsonClient,
}

impl Modrinth {
    pub fn new(base_url: &str) -> Result<Self, ProviderError> {
        Ok(Self {
            client: JsonClient::new("modrinth", base_url)?,
        })
    }

    async fn get(&self, path_and_query: &str) -> Result<Value, ProviderError> {
        self.call(Method::GET, path_and_query, None).await
    }

    async fn call(
        &self,
        method: Method,
        path_and_query: &str,
        body: Option<Value>,
    ) -> Result<Value, ProviderError> {
        self.client.call(method, path_and_query, body, None).await
    }
}

fn quoted(values: &[&str]) -> String {
    let encoded: Vec<String> = values.iter().map(|v| format!("\"{v}\"")).collect();
    format!("[{}]", encoded.join(","))
}

/// Identifiant Modrinth (base62) ou slug : jamais de caractère à échapper dans un chemin.
fn checked_id(id: &str) -> Result<&str, ProviderError> {
    let valid = !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if valid {
        Ok(id)
    } else {
        Err(ProviderError::NotFound)
    }
}

/// Icône de loader gardée si c'est un SVG de taille raisonnable.
fn loader_icon(icon: String) -> Option<String> {
    (icon.len() <= MAX_LOADER_ICON_BYTES && icon.trim_start().starts_with("<svg")).then_some(icon)
}

fn parse<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, ProviderError> {
    serde_json::from_value(value).map_err(|e| ProviderError::Unavailable(e.to_string()))
}

#[derive(Deserialize)]
struct RawHit {
    project_id: String,
    slug: String,
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    icon_url: Option<String>,
    #[serde(default)]
    downloads: u64,
}

#[derive(Deserialize)]
struct RawLoader {
    name: String,
    #[serde(default)]
    icon: String,
}

#[derive(Deserialize)]
struct RawCategory {
    name: String,
    project_type: String,
    #[serde(default)]
    icon: String,
}

/// Nom lisible d'une catégorie Modrinth : `game-mechanics` → `Game mechanics`.
fn category_label(name: &str) -> String {
    let spaced = name.replace('-', " ");
    let mut chars = spaced.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// Index de tri de Modrinth.
fn sort_index(sort: SearchSort) -> &'static str {
    match sort {
        SearchSort::Relevance => "relevance",
        SearchSort::Downloads => "downloads",
        SearchSort::Updated => "updated",
        SearchSort::Newest => "newest",
    }
}

/// Requête de recherche : type, côté serveur, version du jeu, loaders acceptés et catégories
/// choisies en facettes (les tableaux s'ajoutent ; à l'intérieur, l'un ou l'autre), tri par index.
fn search_path(query: &SearchQuery) -> String {
    let mut facets = vec![
        quoted(&[&format!("project_type:{}", query.compat.project_type)]),
        // Seulement ce qui tourne côté serveur.
        quoted(&["server_side:required", "server_side:optional"]),
    ];
    // Sans version (modpack) : le pack impose la sienne.
    if !query.compat.game_version.is_empty() {
        facets.push(quoted(&[&format!(
            "versions:{}",
            query.compat.game_version
        )]));
    }
    if let Some(loader) = &query.compat.loader {
        // Un tableau intérieur de facettes vaut « l'un ou l'autre ».
        let categories: Vec<String> = accepted_loaders(loader)
            .into_iter()
            .map(|l| format!("categories:{l}"))
            .collect();
        let categories: Vec<&str> = categories.iter().map(String::as_str).collect();
        facets.push(quoted(&categories));
    }
    // Catégories choisies : toutes, comme chez CurseForge (un tableau chacune).
    for category in &query.categories {
        facets.push(quoted(&[&format!("categories:{category}")]));
    }
    format!(
        "/search?query={}&facets={}&index={}&offset={}&limit={}",
        encode(&query.text),
        encode(&format!("[{}]", facets.join(","))),
        sort_index(query.sort),
        query.offset,
        query.limit.min(50),
    )
}

#[derive(Deserialize)]
struct RawSearch {
    hits: Vec<RawHit>,
    total_hits: u64,
}

#[derive(Deserialize)]
struct RawProject {
    id: String,
    title: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    icon_url: Option<String>,
}

#[derive(Deserialize)]
struct RawFile {
    url: String,
    filename: String,
    hashes: HashMap<String, String>,
    size: u64,
    #[serde(default)]
    primary: bool,
}

#[derive(Deserialize)]
struct RawDependency {
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    version_id: Option<String>,
    dependency_type: String,
}

#[derive(Deserialize)]
struct RawVersion {
    id: String,
    project_id: String,
    name: String,
    version_number: String,
    date_published: String,
    #[serde(default)]
    loaders: Vec<String>,
    #[serde(default)]
    game_versions: Vec<String>,
    #[serde(default)]
    files: Vec<RawFile>,
    #[serde(default)]
    dependencies: Vec<RawDependency>,
}

impl From<RawVersion> for ProviderVersion {
    fn from(raw: RawVersion) -> Self {
        Self {
            id: raw.id,
            project_id: raw.project_id,
            name: raw.name,
            version_number: raw.version_number,
            published: raw.date_published,
            loaders: raw.loaders,
            game_versions: raw.game_versions,
            files: raw
                .files
                .into_iter()
                .filter_map(|file| {
                    Some(ProviderFile {
                        hash: ContentHash::Sha512(file.hashes.get("sha512")?.clone()),
                        url: file.url,
                        file_name: file.filename,
                        size: file.size,
                        primary: file.primary,
                    })
                })
                .collect(),
            dependencies: raw
                .dependencies
                .into_iter()
                .map(|dep| Dependency {
                    project_id: dep.project_id,
                    version_id: dep.version_id,
                    required: dep.dependency_type == "required",
                })
                .collect(),
        }
    }
}

#[derive(Deserialize)]
struct RawGameVersion {
    version: String,
    version_type: String,
}

impl ContentProvider for Modrinth {
    fn search(&self, query: &SearchQuery) -> BoxFuture<'_, Result<SearchPage, ProviderError>> {
        let query = query.clone();
        Box::pin(async move {
            let path = search_path(&query);
            let raw: RawSearch = parse(self.get(&path).await?)?;
            Ok(SearchPage {
                total: raw.total_hits,
                hits: raw
                    .hits
                    .into_iter()
                    .map(|hit| ProjectHit {
                        page_url: Some(format!("https://modrinth.com/project/{}", hit.slug)),
                        download_allowed: true,
                        project_id: hit.project_id,
                        slug: hit.slug,
                        title: hit.title,
                        description: hit.description,
                        author: hit.author,
                        icon_url: hit.icon_url,
                        downloads: hit.downloads,
                    })
                    .collect(),
            })
        })
    }

    fn project(&self, project_id: &str) -> BoxFuture<'_, Result<Project, ProviderError>> {
        let project_id = project_id.to_owned();
        Box::pin(async move {
            let raw: RawProject = parse(
                self.get(&format!("/project/{}", checked_id(&project_id)?))
                    .await?,
            )?;
            Ok(Project {
                id: raw.id,
                title: raw.title,
                icon_url: raw.icon_url,
                description: raw.description,
                download_allowed: true,
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
            let mut filters = Vec::new();
            if !compat.game_version.is_empty() {
                filters.push(format!(
                    "game_versions={}",
                    encode(&quoted(&[&compat.game_version]))
                ));
            }
            if let Some(loader) = &compat.loader {
                filters.push(format!(
                    "loaders={}",
                    encode(&quoted(&accepted_loaders(loader)))
                ));
            }
            let path = format!(
                "/project/{}/version?{}",
                checked_id(&project_id)?,
                filters.join("&")
            );
            let raw: Vec<RawVersion> = parse(self.get(&path).await?)?;
            Ok(raw.into_iter().map(ProviderVersion::from).collect())
        })
    }

    fn version(&self, version_id: &str) -> BoxFuture<'_, Result<ProviderVersion, ProviderError>> {
        let version_id = version_id.to_owned();
        Box::pin(async move {
            let raw: RawVersion = parse(
                self.get(&format!("/version/{}", checked_id(&version_id)?))
                    .await?,
            )?;
            Ok(raw.into())
        })
    }

    /// Par empreinte : `/version_files/update` donne la dernière version de chaque fichier.
    fn latest(
        &self,
        installed: &[InstalledRef],
        compat: &Compat,
    ) -> BoxFuture<'_, Result<HashMap<String, ProviderVersion>, ProviderError>> {
        let hashes: Vec<String> = installed.iter().filter_map(|i| i.sha512.clone()).collect();
        let compat = compat.clone();
        Box::pin(async move {
            if hashes.is_empty() {
                return Ok(HashMap::new());
            }
            let mut body = json!({
                "hashes": hashes,
                "algorithm": "sha512",
                "game_versions": [compat.game_version],
            });
            if let Some(loader) = &compat.loader {
                body["loaders"] = json!(accepted_loaders(loader));
            }
            let raw: HashMap<String, RawVersion> = parse(
                self.call(Method::POST, "/version_files/update", Some(body))
                    .await?,
            )?;
            Ok(raw
                .into_values()
                .map(|version| (version.project_id.clone(), version.into()))
                .collect())
        })
    }

    fn latest_game_version(&self) -> BoxFuture<'_, Result<String, ProviderError>> {
        Box::pin(async move {
            let raw: Vec<RawGameVersion> = parse(self.get("/tag/game_version").await?)?;
            raw.into_iter()
                .find(|v| v.version_type == "release")
                .map(|v| v.version)
                .ok_or(ProviderError::NotFound)
        })
    }

    fn fetch(&self, url: &str, max_bytes: usize) -> BoxFuture<'_, Result<Vec<u8>, ProviderError>> {
        let url = url.to_owned();
        Box::pin(async move {
            if !self.is_allowed_download(&url) {
                return Err(ProviderError::NotFound);
            }
            let uri: Uri = url.parse().map_err(|_| ProviderError::NotFound)?;
            let (status, bytes) = timeout(
                Duration::from_secs(120),
                self.client
                    .request(Method::GET, &uri, None, None, max_bytes),
            )
            .await
            .map_err(|_| ProviderError::Unavailable("download timed out".to_owned()))??;
            match status {
                StatusCode::OK => Ok(bytes.to_vec()),
                StatusCode::NOT_FOUND => Err(ProviderError::NotFound),
                other => Err(ProviderError::Unavailable(format!("cdn answered {other}"))),
            }
        })
    }

    fn loaders(&self) -> BoxFuture<'_, Result<Vec<Loader>, ProviderError>> {
        Box::pin(async move {
            let raw: Vec<RawLoader> = parse(self.get("/tag/loader").await?)?;
            Ok(raw
                .into_iter()
                .map(|loader| Loader {
                    icon_svg: loader_icon(loader.icon),
                    name: loader.name,
                })
                .collect())
        })
    }

    fn categories(&self, compat: &Compat) -> BoxFuture<'_, Result<Vec<Category>, ProviderError>> {
        let project_type = compat.project_type.clone();
        Box::pin(async move {
            let raw: Vec<RawCategory> = parse(self.get("/tag/category").await?)?;
            let mut categories: Vec<Category> = raw
                .into_iter()
                .filter(|c| c.project_type == project_type)
                .map(|c| Category {
                    name: category_label(&c.name),
                    icon_svg: loader_icon(c.icon),
                    icon_url: None,
                    id: c.name,
                })
                .collect();
            categories.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(categories)
        })
    }

    fn is_allowed_pack_download(&self, url: &str) -> bool {
        url.parse::<Uri>().is_ok_and(|uri| {
            uri.scheme_str() == Some("https")
                && uri.port().is_none()
                && uri.host().is_some_and(|host| PACK_HOSTS.contains(&host))
        })
    }

    fn is_allowed_download(&self, url: &str) -> bool {
        url.parse::<Uri>().is_ok_and(|uri| {
            uri.scheme_str() == Some("https")
                && uri.host() == Some(DOWNLOAD_HOST)
                && uri.port().is_none()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(sort: SearchSort, categories: &[&str]) -> SearchQuery {
        SearchQuery {
            text: "map".to_owned(),
            compat: Compat {
                project_type: "mod".to_owned(),
                game: Some("minecraft".to_owned()),
                class: None,
                loader: Some("quilt".to_owned()),
                game_version: "1.21.4".to_owned(),
            },
            offset: 20,
            limit: 20,
            sort,
            categories: categories.iter().map(|c| (*c).to_owned()).collect(),
        }
    }

    fn decoded(path: &str) -> String {
        let mut out = Vec::new();
        let bytes = path.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' {
                out.push(u8::from_str_radix(&path[i + 1..i + 3], 16).unwrap());
                i += 3;
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        }
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn the_search_sorts_and_filters_by_every_chosen_category() {
        let path = decoded(&search_path(&query(
            SearchSort::Updated,
            &["magic", "storage"],
        )));
        assert!(path.contains("&index=updated&"), "{path}");
        assert!(
            path.contains(r#"["categories:magic"],["categories:storage"]"#),
            "{path}"
        );
        assert!(
            path.contains(r#"["categories:quilt","categories:fabric"]"#),
            "{path}"
        );
        assert!(path.contains(r#"["versions:1.21.4"]"#), "{path}");
        let plain = decoded(&search_path(&query(SearchSort::Relevance, &[])));
        assert!(plain.contains("&index=relevance&"), "{plain}");
        assert_eq!(plain.matches("categories:").count(), 2, "{plain}");
    }

    #[test]
    fn category_names_read_as_labels() {
        assert_eq!(category_label("game-mechanics"), "Game mechanics");
        assert_eq!(category_label("magic"), "Magic");
        assert_eq!(category_label(""), "");
    }

    #[test]
    fn loader_icons_are_kept_only_as_small_svg() {
        let svg = r#"<svg viewBox="0 0 24 24"><path d="M0 0"/></svg>"#.to_owned();
        assert_eq!(loader_icon(svg.clone()), Some(svg));
        assert_eq!(loader_icon(String::new()), None);
        assert_eq!(loader_icon("<script>alert(1)</script>".to_owned()), None);
        let huge = format!("<svg>{}</svg>", "x".repeat(MAX_LOADER_ICON_BYTES));
        assert_eq!(loader_icon(huge), None);
    }
}
