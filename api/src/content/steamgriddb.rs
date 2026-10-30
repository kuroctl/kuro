//! SteamGridDB (ADR-0040) : images de jeux de la communauté, cherchées par identifiant Steam pour
//! le Prefab Studio. Source à clé d'API (réglée dans le panel, ADR-0029), passée en en-tête
//! `Authorization`, jamais dans l'adresse. Filtres fixes : images statiques, ni NSFW, ni humour,
//! ni risque d'épilepsie. Images lues seulement sur le CDN de SteamGridDB, sans redirection.

use std::fmt::Debug;
use std::sync::{Arc, PoisonError, RwLock};

use futures_util::future::BoxFuture;
use hyper::header::AUTHORIZATION;
use hyper::{Method, StatusCode, Uri};
use kuro_prefab::assets::{AssetKind, MAX_ASSET_BYTES};
use kuro_prefab::media::target;
use serde::Deserialize;
use serde_json::Value;

use super::client::{Header, JsonClient};
use super::provider::ProviderError;

pub const DEFAULT_BASE_URL: &str = "https://www.steamgriddb.com/api/v2";
/// Hôtes d'où les images sont lues.
const CDN_HOSTS: [&str; 2] = ["cdn2.steamgriddb.com", "cdn.steamgriddb.com"];
/// Images essayées par type, de la mieux notée à la moins bien notée.
const CANDIDATES: usize = 3;
const FILTERS: &str = "types=static&nsfw=false&humor=false&epilepsy=false";

/// Image proposée par SteamGridDB.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridImage {
    pub id: u64,
    pub url: String,
    pub author: Option<String>,
}

pub trait SteamGrid: Send + Sync + Debug {
    /// Une clé est réglée.
    fn configured(&self) -> bool;
    fn set_key(&self, key: Option<String>);
    /// Vérifie une clé auprès de SteamGridDB, sans la garder.
    fn verify_key(&self, key: &str) -> BoxFuture<'_, Result<(), ProviderError>>;
    /// Images convenables pour ce type, la mieux notée d'abord (vide si aucune).
    fn images(
        &self,
        app: u32,
        kind: AssetKind,
    ) -> BoxFuture<'_, Result<Vec<GridImage>, ProviderError>>;
    /// Contenu d'une image du CDN, 2 Mio au plus.
    fn fetch(&self, url: &str) -> BoxFuture<'_, Result<Vec<u8>, ProviderError>>;
}

#[derive(Debug, Clone)]
pub struct SteamGridDb {
    client: JsonClient,
    key: Arc<RwLock<Option<String>>>,
}

impl SteamGridDb {
    pub fn new(base_url: &str) -> Result<Self, ProviderError> {
        Ok(Self {
            client: JsonClient::new("steamgriddb", base_url)?,
            key: Arc::default(),
        })
    }

    fn header(key: &str) -> Header {
        (AUTHORIZATION, format!("Bearer {key}"))
    }

    async fn get(&self, path: &str, key: &str) -> Result<Value, ProviderError> {
        self.client
            .call(Method::GET, path, None, Some(&Self::header(key)))
            .await
    }
}

#[derive(Deserialize)]
struct Images {
    #[serde(default)]
    data: Vec<RawImage>,
}

#[derive(Deserialize)]
struct RawImage {
    id: u64,
    #[serde(default)]
    score: i64,
    url: String,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
    #[serde(default)]
    author: Option<RawAuthor>,
}

#[derive(Deserialize)]
struct RawAuthor {
    name: Option<String>,
}

/// Chemin de recherche d'un type d'image : collection et filtres propres au type.
fn query(app: u32, kind: AssetKind) -> String {
    match kind {
        AssetKind::Thumbnail => format!("/grids/steam/{app}?dimensions=600x900&{FILTERS}"),
        AssetKind::Cover => format!("/heroes/steam/{app}?dimensions=1920x620&{FILTERS}"),
        AssetKind::Icon => format!("/icons/steam/{app}?mimes=image/png&{FILTERS}"),
    }
}

/// Images retenues : adresse du CDN valide, icône carrée et assez grande ; la mieux notée
/// d'abord.
fn select(kind: AssetKind, mut raw: Vec<RawImage>) -> Vec<GridImage> {
    let (min, _) = target(AssetKind::Icon);
    raw.retain(|image| {
        is_cdn_url(&image.url)
            && (kind != AssetKind::Icon || (image.width == image.height && image.width >= min))
    });
    raw.sort_by_key(|image| std::cmp::Reverse(image.score));
    raw.into_iter()
        .take(CANDIDATES)
        .map(|image| GridImage {
            id: image.id,
            url: image.url,
            author: image.author.and_then(|author| author.name),
        })
        .collect()
}

impl SteamGrid for SteamGridDb {
    fn configured(&self) -> bool {
        self.key
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }

    fn set_key(&self, key: Option<String>) {
        *self.key.write().unwrap_or_else(PoisonError::into_inner) = key;
    }

    fn verify_key(&self, key: &str) -> BoxFuture<'_, Result<(), ProviderError>> {
        let key = key.to_owned();
        Box::pin(async move {
            // Jeu connu (Counter-Strike 2) : 200 avec une clé valide, 401 sinon.
            self.get("/games/steam/730", &key).await.map(|_| ())
        })
    }

    fn images(
        &self,
        app: u32,
        kind: AssetKind,
    ) -> BoxFuture<'_, Result<Vec<GridImage>, ProviderError>> {
        Box::pin(async move {
            let key = self
                .key
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
                .ok_or(ProviderError::NotConfigured)?;
            let images: Images = match self.get(&query(app, kind), &key).await {
                Ok(value) => serde_json::from_value(value)
                    .map_err(|e| ProviderError::Unavailable(e.to_string()))?,
                // Jeu inconnu de SteamGridDB.
                Err(ProviderError::NotFound) => return Ok(Vec::new()),
                Err(other) => return Err(other),
            };
            Ok(select(kind, images.data))
        })
    }

    fn fetch(&self, url: &str) -> BoxFuture<'_, Result<Vec<u8>, ProviderError>> {
        let url = url.to_owned();
        Box::pin(async move {
            if !is_cdn_url(&url) {
                return Err(ProviderError::NotFound);
            }
            let uri: Uri = url.parse().map_err(|_| ProviderError::NotFound)?;
            let (status, bytes) = self
                .client
                .request(Method::GET, &uri, None, None, MAX_ASSET_BYTES as usize)
                .await?;
            match status {
                StatusCode::OK => Ok(bytes.to_vec()),
                StatusCode::NOT_FOUND | StatusCode::FORBIDDEN => Err(ProviderError::NotFound),
                other => Err(ProviderError::Unavailable(format!(
                    "steamgriddb cdn answered {other}"
                ))),
            }
        })
    }
}

/// `https://<hôte du CDN>/<segments simples>.<png|jpg|jpeg|webp>`, sans port, requête ni `..`.
fn is_cdn_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let Some((host, path)) = rest.split_once('/') else {
        return false;
    };
    if !CDN_HOSTS.contains(&host) {
        return false;
    }
    let segments: Vec<&str> = path.split('/').collect();
    let Some((file, dirs)) = segments.split_last() else {
        return false;
    };
    let simple = |segment: &str| {
        !segment.is_empty()
            && segment
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    };
    let Some((name, extension)) = file.rsplit_once('.') else {
        return false;
    };
    dirs.iter().all(|dir| simple(dir))
        && simple(name)
        && matches!(
            extension.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "webp"
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(id: u64, score: i64, size: u32, url: &str) -> RawImage {
        RawImage {
            id,
            score,
            url: url.to_owned(),
            width: size,
            height: size,
            author: Some(RawAuthor {
                name: Some(format!("author{id}")),
            }),
        }
    }

    #[test]
    fn the_best_rated_valid_images_come_first() {
        let cdn = |name: &str| format!("https://cdn2.steamgriddb.com/icon/{name}.png");
        let found = select(
            AssetKind::Icon,
            vec![
                raw(1, 3, 256, &cdn("a")),
                raw(2, 9, 256, &cdn("b")),
                // Trop petite, ou hors du CDN : écartées.
                raw(3, 50, 32, &cdn("c")),
                raw(4, 40, 256, "https://evil.example/icon/d.png"),
                raw(5, 7, 512, &cdn("e")),
                raw(6, 1, 512, &cdn("f")),
            ],
        );
        let ids: Vec<u64> = found.iter().map(|image| image.id).collect();
        assert_eq!(ids, vec![2, 5, 1]);
        assert_eq!(found[0].author.as_deref(), Some("author2"));
    }

    #[test]
    fn a_non_square_icon_is_dropped_but_not_a_hero() {
        let mut wide = raw(1, 1, 256, "https://cdn2.steamgriddb.com/hero/x.png");
        wide.width = 1920;
        wide.height = 620;
        assert!(select(AssetKind::Icon, vec![wide]).is_empty());
        let mut hero = raw(1, 1, 256, "https://cdn2.steamgriddb.com/hero/x.png");
        hero.width = 1920;
        hero.height = 620;
        assert_eq!(select(AssetKind::Cover, vec![hero]).len(), 1);
    }

    #[test]
    fn only_steamgriddb_cdn_images_are_fetched() {
        for good in [
            "https://cdn2.steamgriddb.com/icon/0123abcd.png",
            "https://cdn2.steamgriddb.com/file/sgdb-cdn/grid/0123abcd.webp",
            "https://cdn.steamgriddb.com/hero/ab_cd-ef.JPG",
        ] {
            assert!(is_cdn_url(good), "{good}");
        }
        for bad in [
            "http://cdn2.steamgriddb.com/icon/a.png",
            "https://cdn2.steamgriddb.com:8443/icon/a.png",
            "https://evil.example/icon/a.png",
            "https://cdn2.steamgriddb.com.evil.example/icon/a.png",
            "https://cdn2.steamgriddb.com/icon/../a.png",
            "https://cdn2.steamgriddb.com/icon/a.png?x=1",
            "https://cdn2.steamgriddb.com/icon/a.svg",
            "https://cdn2.steamgriddb.com/icon/a.ico",
            "https://cdn2.steamgriddb.com/icon//a.png",
        ] {
            assert!(!is_cdn_url(bad), "{bad}");
        }
    }

    #[test]
    fn each_kind_asks_for_its_collection_and_filters() {
        assert_eq!(
            query(730, AssetKind::Icon),
            "/icons/steam/730?mimes=image/png&types=static&nsfw=false&humor=false&epilepsy=false"
        );
        assert!(
            query(730, AssetKind::Thumbnail).starts_with("/grids/steam/730?dimensions=600x900&")
        );
        assert!(query(730, AssetKind::Cover).starts_with("/heroes/steam/730?dimensions=1920x620&"));
    }
}
