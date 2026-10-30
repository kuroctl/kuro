//! Magasin Steam (ADR-0039) : images d'un jeu pour le Prefab Studio. Les chemins viennent de
//! l'API publique du magasin (`IStoreBrowseService/GetItems`, sans clé) : les jeux récents rangent
//! leurs images sous un chemin haché, que l'ancien chemin fixe ne trouve pas. L'icône est celle
//! de la communauté (carrée : 184 px quand Steam la publie, sinon 32 px), et le logo seulement
//! pour un jeu qui n'en a pas. L'icône du raccourci bureau (`clienticon`) n'est donnée que par le
//! client Steam, pas par une API web. Images lues sur un seul hôte du CDN, chemin validé, sans
//! suivre de redirection.

use std::fmt::Debug;

use futures_util::future::BoxFuture;
use hyper::{Method, StatusCode, Uri};
use kuro_prefab::assets::{AssetKind, MAX_ASSET_BYTES};
use serde::Deserialize;
use serde_json::json;

use super::client::{JsonClient, encode};
use super::provider::ProviderError;

/// Hôte des images du magasin et de la communauté.
const CDN_URL: &str = "https://shared.akamai.steamstatic.com";
const STORE_PREFIX: &str = "store_item_assets/steam/apps/";
const COMMUNITY_PREFIX: &str = "community_assets/images/apps/";

/// Jeu du magasin : son nom et, par image, les chemins à essayer dans l'ordre (sous [`CDN_URL`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreGame {
    pub name: String,
    pub images: Vec<(AssetKind, Vec<String>)>,
}

pub trait SteamStore: Send + Sync + Debug {
    /// Jeu visible sur le magasin ; `NotFound` pour une application inconnue (un serveur
    /// dédié, par exemple).
    fn game(&self, app: u32) -> BoxFuture<'_, Result<StoreGame, ProviderError>>;
    /// Image du CDN, 2 Mio au plus ; `NotFound` si elle n'existe pas.
    fn image(&self, path: &str) -> BoxFuture<'_, Result<Vec<u8>, ProviderError>>;
}

#[derive(Debug, Clone)]
pub struct SteamStoreClient {
    api: JsonClient,
    cdn: JsonClient,
}

impl SteamStoreClient {
    /// `api_url` : API Web de Steam (celle du Workshop, `content.steam_url`).
    pub fn new(api_url: &str) -> Result<Self, ProviderError> {
        Ok(Self {
            api: JsonClient::new("steam store", api_url)?,
            cdn: JsonClient::new("steam cdn", CDN_URL)?,
        })
    }
}

#[derive(Deserialize)]
struct Items {
    response: ItemsResponse,
}

#[derive(Deserialize)]
struct ItemsResponse {
    #[serde(default)]
    store_items: Vec<Item>,
}

#[derive(Deserialize)]
struct Item {
    /// 1 : trouvé.
    success: u32,
    #[serde(default)]
    name: String,
    #[serde(default)]
    assets: Option<Assets>,
}

#[derive(Deserialize)]
struct Assets {
    /// `steam/apps/<app>/${FILENAME}?t=<horodatage>`.
    asset_url_format: String,
    library_capsule: Option<String>,
    library_hero: Option<String>,
    /// Empreinte de l'icône carrée de la communauté.
    community_icon: Option<String>,
}

impl SteamStore for SteamStoreClient {
    fn game(&self, app: u32) -> BoxFuture<'_, Result<StoreGame, ProviderError>> {
        Box::pin(async move {
            let input = json!({
                "ids": [{ "appid": app }],
                "context": { "language": "english", "country_code": "US" },
                "data_request": { "include_assets": true },
            });
            let path = format!(
                "/IStoreBrowseService/GetItems/v1?input_json={}",
                encode(&input.to_string())
            );
            let value = self.api.call(Method::GET, &path, None, None).await?;
            let items: Items = serde_json::from_value(value)
                .map_err(|e| ProviderError::Unavailable(e.to_string()))?;
            let item = items
                .response
                .store_items
                .into_iter()
                .next()
                .filter(|item| item.success == 1 && !item.name.is_empty())
                .ok_or(ProviderError::NotFound)?;
            Ok(StoreGame {
                name: item.name,
                images: images(app, item.assets.as_ref()),
            })
        })
    }

    fn image(&self, path: &str) -> BoxFuture<'_, Result<Vec<u8>, ProviderError>> {
        let path = path.to_owned();
        Box::pin(async move {
            if !is_image_path(&path) {
                return Err(ProviderError::NotFound);
            }
            let uri: Uri = format!("{CDN_URL}/{path}")
                .parse()
                .map_err(|_| ProviderError::NotFound)?;
            let (status, bytes) = self
                .cdn
                .request(Method::GET, &uri, None, None, MAX_ASSET_BYTES as usize)
                .await?;
            match status {
                StatusCode::OK => Ok(bytes.to_vec()),
                StatusCode::NOT_FOUND | StatusCode::FORBIDDEN => Err(ProviderError::NotFound),
                other => Err(ProviderError::Unavailable(format!(
                    "steam cdn answered {other}"
                ))),
            }
        })
    }
}

/// Images d'un jeu : jaquette (`library_capsule`), bannière (`library_hero`), icône. Un chemin
/// annoncé hors du dossier du jeu, ou mal formé, est écarté.
fn images(app: u32, assets: Option<&Assets>) -> Vec<(AssetKind, Vec<String>)> {
    let dir = format!("{STORE_PREFIX}{app}/");
    let mut out = Vec::new();
    let mut icon = Vec::new();
    if let Some(assets) = assets {
        let format = assets
            .asset_url_format
            .split('?')
            .next()
            .unwrap_or_default();
        for (kind, file) in [
            (AssetKind::Thumbnail, &assets.library_capsule),
            (AssetKind::Cover, &assets.library_hero),
        ] {
            if let Some(file) = file {
                let path = format!("store_item_assets/{}", format.replace("${FILENAME}", file));
                if path.starts_with(&dir) && is_image_path(&path) {
                    out.push((kind, vec![path]));
                }
            }
        }
        if let Some(hash) = &assets.community_icon {
            for suffix in ["_full", ""] {
                let path = format!("{COMMUNITY_PREFIX}{app}/{hash}{suffix}.jpg");
                if is_image_path(&path) {
                    icon.push(path);
                }
            }
        }
    }
    if icon.is_empty() {
        icon.push(format!("{dir}logo.png"));
    }
    out.push((AssetKind::Icon, icon));
    out
}

/// `store_item_assets/steam/apps/<app>/[<sha1>/]<fichier>.jpg|png` ou
/// `community_assets/images/apps/<app>/<sha1>[_full].jpg`, rien d'autre.
fn is_image_path(path: &str) -> bool {
    let is_app = |app: &str| !app.is_empty() && app.bytes().all(|b| b.is_ascii_digit());
    let is_hash = |hash: &str| hash.len() == 40 && hash.bytes().all(|b| b.is_ascii_hexdigit());
    if let Some(rest) = path.strip_prefix(COMMUNITY_PREFIX) {
        let Some((app, file)) = rest.split_once('/') else {
            return false;
        };
        let hash = file
            .strip_suffix(".jpg")
            .map(|name| name.strip_suffix("_full").unwrap_or(name))
            .unwrap_or_default();
        return is_app(app) && is_hash(hash);
    }
    let Some(rest) = path.strip_prefix(STORE_PREFIX) else {
        return false;
    };
    let mut parts = rest.split('/');
    let app = parts.next().unwrap_or_default();
    let mut tail: Vec<&str> = parts.collect();
    let Some(file) = tail.pop() else {
        return false;
    };
    let hash_ok = match tail.as_slice() {
        [] => true,
        [hash] => is_hash(hash),
        _ => false,
    };
    let name = file
        .strip_suffix(".jpg")
        .or_else(|| file.strip_suffix(".png"))
        .unwrap_or_default();
    is_app(app)
        && hash_ok
        && !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
        && !name.contains("..")
}

#[cfg(test)]
mod tests {
    use super::*;

    const ICON: &str = "2f64c9a826e2c6cf3253fea4834c2e612db09143";

    fn assets(format: &str, capsule: &str, hero: &str, icon: Option<&str>) -> Assets {
        Assets {
            asset_url_format: format.to_owned(),
            library_capsule: Some(capsule.to_owned()),
            library_hero: Some(hero.to_owned()),
            community_icon: icon.map(str::to_owned),
        }
    }

    #[test]
    fn hashed_paths_and_the_community_icon_are_taken_from_the_store() {
        let hash = "3dee2eb67bca0660c5ca3c9e1cfbdfc1063333ba";
        let found = images(
            892970,
            Some(&assets(
                "steam/apps/892970/${FILENAME}?t=1791544831",
                &format!("{hash}/library_capsule.jpg"),
                "library_hero.jpg",
                Some(ICON),
            )),
        );
        assert_eq!(
            found,
            vec![
                (
                    AssetKind::Thumbnail,
                    vec![format!(
                        "store_item_assets/steam/apps/892970/{hash}/library_capsule.jpg"
                    )]
                ),
                (
                    AssetKind::Cover,
                    vec!["store_item_assets/steam/apps/892970/library_hero.jpg".to_owned()]
                ),
                (
                    AssetKind::Icon,
                    vec![
                        format!("community_assets/images/apps/892970/{ICON}_full.jpg"),
                        format!("community_assets/images/apps/892970/{ICON}.jpg"),
                    ]
                ),
            ]
        );
    }

    #[test]
    fn without_a_community_icon_the_logo_stands_in() {
        let found = images(
            730,
            Some(&assets(
                "steam/apps/440/${FILENAME}",
                "library_600x900.jpg",
                "library_hero.jpg",
                None,
            )),
        );
        // Chemins annoncés hors du dossier du jeu : écartés.
        assert_eq!(
            found,
            vec![(
                AssetKind::Icon,
                vec!["store_item_assets/steam/apps/730/logo.png".to_owned()]
            )]
        );
    }

    #[test]
    fn only_steam_image_paths_are_fetched() {
        for good in [
            "store_item_assets/steam/apps/730/library_600x900.jpg",
            &format!("community_assets/images/apps/730/{ICON}.jpg"),
            &format!("community_assets/images/apps/730/{ICON}_full.jpg"),
        ] {
            assert!(is_image_path(good), "{good}");
        }
        for bad in [
            "store_item_assets/steam/apps/730/../../x.jpg",
            "store_item_assets/steam/apps/730/a/b/c.jpg",
            "store_item_assets/steam/apps/730/logo.svg",
            "store_item_assets/steam/apps/730/notahash/logo.png",
            "store_item_assets/steam/apps/x/logo.png",
            "store_item_assets/steam/apps/730/.jpg",
            "steam/apps/730/logo.png",
            "community_assets/images/apps/730/short.jpg",
            &format!("community_assets/images/apps/730/{ICON}.png"),
            &format!("community_assets/images/apps/730/x/{ICON}.jpg"),
            "https://evil.example/logo.png",
        ] {
            assert!(!is_image_path(bad), "{bad}");
        }
    }
}
