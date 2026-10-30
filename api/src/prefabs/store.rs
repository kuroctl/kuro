//! Stores de prefabs (ADR-0042) : un dépôt de prefabs publie un index signé (minisign) et une
//! archive `.kprefab` par version ; l'API lit l'index, en vérifie la signature avec la clé
//! publique configurée, et installe une version en vérifiant la somme SHA-256 de son archive
//! contre l'index, puis par le même chemin qu'une archive importée (source `store`).
//!
//! L'accès HTTP est derrière [`StoreFetcher`] : HTTPS en production, simulé dans les tests.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use futures_util::future::BoxFuture;
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::header::{ACCEPT, AUTHORIZATION, HOST, USER_AGENT};
use hyper::{Method, Request, StatusCode, Uri};
use kuro_prefab::files::{ArchiveFiles, MAX_ARCHIVE_BYTES};
use kuro_prefab::store::{INDEX_FILE, IndexEntry, StoreIndex};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use super::application::{self as prefabs, ImportAs, Imported, PrefabsError};
use super::infra::PrefabRepository;
use crate::audit::domain::AuditContext;
use crate::config::StoreConfig;
use crate::http::outbound::{Https, Target};

/// Durée pendant laquelle un index lu est gardé (le panel peut forcer une relecture).
const INDEX_TTL: Duration = Duration::from_secs(600);
/// Taille maximale d'un index et de sa signature.
const MAX_INDEX_BYTES: usize = 4 * 1024 * 1024;
const MAX_SIGNATURE_BYTES: usize = 4 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    Unavailable(String),
}

/// Lecture d'une adresse d'un store : `GET`, `Authorization: Bearer <token>` s'il y en a un,
/// réponse bornée à `max` octets.
pub trait StoreFetcher: Send + Sync + std::fmt::Debug {
    fn fetch(
        &self,
        url: &str,
        token: Option<&str>,
        max: usize,
    ) -> BoxFuture<'_, Result<Vec<u8>, FetchError>>;
}

/// Lecteur HTTPS (autorités publiques, HTTP/1.1), comme les sources de contenu.
#[derive(Debug, Clone)]
pub struct HttpFetcher {
    https: Https,
}

impl HttpFetcher {
    pub fn new() -> Result<Self, rustls::Error> {
        Ok(Self {
            https: Https::new()?,
        })
    }
}

impl StoreFetcher for HttpFetcher {
    fn fetch(
        &self,
        url: &str,
        token: Option<&str>,
        max: usize,
    ) -> BoxFuture<'_, Result<Vec<u8>, FetchError>> {
        let url = url.to_owned();
        let token = token.map(str::to_owned);
        Box::pin(async move {
            let unavailable = |e: &dyn std::fmt::Display| FetchError::Unavailable(e.to_string());
            let uri: Uri = url.parse().map_err(|e| unavailable(&e))?;
            if uri.scheme_str() != Some("https") {
                return Err(unavailable(&"store url must be https"));
            }
            let host = uri
                .host()
                .ok_or_else(|| unavailable(&"no host"))?
                .to_owned();
            let port = uri.port_u16().unwrap_or(443);
            let mut request = Request::builder()
                .method(Method::GET)
                .uri(uri.path_and_query().map_or("/", |p| p.as_str()))
                .header(HOST, uri.authority().map_or(host.as_str(), |a| a.as_str()))
                .header(
                    USER_AGENT,
                    concat!(
                        "kuro-api/",
                        env!("CARGO_PKG_VERSION"),
                        " (github.com/kuroctl/kuro)"
                    ),
                )
                .header(ACCEPT, "*/*");
            if let Some(token) = token {
                request = request.header(AUTHORIZATION, format!("Bearer {token}"));
            }
            let request = request
                .body(Full::new(Bytes::new()))
                .map_err(|e| unavailable(&e))?;
            let (status, bytes) = self
                .https
                .send(
                    &host,
                    Target::Host(host.clone(), port),
                    request,
                    max,
                    TIMEOUT,
                )
                .await
                .map_err(|e| unavailable(&e))?;
            match status {
                StatusCode::OK => Ok(bytes.to_vec()),
                StatusCode::NOT_FOUND => Err(FetchError::NotFound),
                other => Err(FetchError::Unavailable(format!("store answered {other}"))),
            }
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("unknown store")]
    UnknownStore,
    /// Store injoignable ou en erreur.
    #[error("store unavailable: {0}")]
    Unavailable(String),
    /// Index absent, illisible ou signé par une autre clé : rien n'en est cru.
    #[error("store index untrusted: {0}")]
    Untrusted(String),
    /// La version demandée n'est pas dans l'index.
    #[error("version not in the store index")]
    NotInIndex,
    /// L'archive téléchargée n'a pas la somme annoncée par l'index.
    #[error("archive does not match the store index")]
    Corrupted,
    #[error(transparent)]
    Prefabs(#[from] PrefabsError),
}

impl From<sqlx::Error> for StoreError {
    fn from(error: sqlx::Error) -> Self {
        Self::Prefabs(error.into())
    }
}

/// Un store vu du panel : ses versions, et l'erreur qui empêche de le lire le cas échéant.
#[derive(Debug, Clone)]
pub struct StoreView {
    pub name: String,
    pub url: String,
    pub error: Option<String>,
    pub prefabs: Vec<StoreEntryView>,
}

#[derive(Debug, Clone)]
pub struct StoreEntryView {
    pub entry: IndexEntry,
    /// `requires.kuro` satisfait par cette API.
    pub compatible: bool,
    /// Déjà au catalogue (quelle qu'en soit la source).
    pub installed: bool,
}

/// Index vérifiés, par nom de store, avec l'instant de leur lecture.
type IndexCache = Arc<Mutex<HashMap<String, (Instant, Arc<StoreIndex>)>>>;

/// Stores configurés, leur lecteur et le cache des index vérifiés.
#[derive(Debug, Clone)]
pub struct StoresState {
    stores: Arc<Vec<StoreConfig>>,
    fetcher: Arc<dyn StoreFetcher>,
    cache: IndexCache,
}

impl StoresState {
    pub fn new(stores: Vec<StoreConfig>, fetcher: Arc<dyn StoreFetcher>) -> Self {
        Self {
            stores: Arc::new(stores),
            fetcher,
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn stores(&self) -> &[StoreConfig] {
        &self.stores
    }

    fn store(&self, name: &str) -> Result<&StoreConfig, StoreError> {
        self.stores
            .iter()
            .find(|store| store.name == name)
            .ok_or(StoreError::UnknownStore)
    }

    fn cache(&self) -> std::sync::MutexGuard<'_, HashMap<String, (Instant, Arc<StoreIndex>)>> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Fichier `path` du store (relatif à sa racine), borné à `max` octets. Un fichier absent
    /// rend l'index non fiable (pas d'index, pas de signature) ; une panne, injoignable.
    async fn fetch_path(
        &self,
        store: &StoreConfig,
        path: &str,
        max: usize,
    ) -> Result<Vec<u8>, StoreError> {
        let url = format!("{}/{path}", store.url.trim_end_matches('/'));
        self.fetcher
            .fetch(&url, store.token.as_deref(), max)
            .await
            .map_err(|e| match e {
                FetchError::NotFound => StoreError::Untrusted(format!("{path} not found")),
                FetchError::Unavailable(reason) => StoreError::Unavailable(reason),
            })
    }

    /// Index du store, vérifié (signature minisign de `index.json.minisig` par la clé publique
    /// configurée) ; gardé [`INDEX_TTL`] sauf `refresh`.
    pub async fn index(
        &self,
        store: &StoreConfig,
        refresh: bool,
    ) -> Result<Arc<StoreIndex>, StoreError> {
        if !refresh
            && let Some((at, index)) = self.cache().get(&store.name)
            && at.elapsed() < INDEX_TTL
        {
            return Ok(index.clone());
        }
        let public_key = minisign_verify::PublicKey::from_base64(&store.public_key)
            .map_err(|e| StoreError::Untrusted(format!("invalid public key: {e}")))?;
        let index_bytes = self.fetch_path(store, INDEX_FILE, MAX_INDEX_BYTES).await?;
        let signature_bytes = self
            .fetch_path(store, &format!("{INDEX_FILE}.minisig"), MAX_SIGNATURE_BYTES)
            .await?;
        let signature_text = String::from_utf8(signature_bytes)
            .map_err(|_| StoreError::Untrusted("signature is not text".to_owned()))?;
        let signature = minisign_verify::Signature::decode(&signature_text)
            .map_err(|e| StoreError::Untrusted(format!("invalid signature: {e}")))?;
        // `allow_legacy` : signatures non préhachées (`Ed`) acceptées, l'index est entier en
        // mémoire ; minisign récent produit des signatures préhachées (`ED`).
        public_key
            .verify(&index_bytes, &signature, true)
            .map_err(|e| StoreError::Untrusted(format!("signature check failed: {e}")))?;
        let index = StoreIndex::parse(&index_bytes).map_err(StoreError::Untrusted)?;
        let index = Arc::new(index);
        self.cache()
            .insert(store.name.clone(), (Instant::now(), index.clone()));
        Ok(index)
    }

    /// Tous les stores et leurs versions, avec compatibilité et présence au catalogue. Un store
    /// en erreur est rendu avec son erreur, les autres restent lisibles.
    pub async fn catalog(
        &self,
        pool: &PgPool,
        refresh: bool,
    ) -> Result<Vec<StoreView>, PrefabsError> {
        let installed: HashSet<(String, String)> = {
            let mut conn = pool.acquire().await?;
            PrefabRepository::list(&mut conn)
                .await?
                .into_iter()
                .map(|record| (record.id, record.version))
                .collect()
        };
        let kuro = semver::Version::parse(env!("CARGO_PKG_VERSION"))
            .map_err(|e| PrefabsError::Corrupted(e.to_string()))?;
        let mut views = Vec::with_capacity(self.stores.len());
        for store in self.stores.iter() {
            let view = match self.index(store, refresh).await {
                Ok(index) => StoreView {
                    name: store.name.clone(),
                    url: store.url.clone(),
                    error: None,
                    prefabs: index
                        .prefabs
                        .iter()
                        .map(|entry| StoreEntryView {
                            compatible: entry.compatible_with(&kuro),
                            installed: installed
                                .contains(&(entry.id.clone(), entry.version.clone())),
                            entry: entry.clone(),
                        })
                        .collect(),
                },
                Err(error) => {
                    tracing::warn!(store = %store.name, %error, "prefab store unreadable");
                    StoreView {
                        name: store.name.clone(),
                        url: store.url.clone(),
                        error: Some(error.to_string()),
                        prefabs: Vec::new(),
                    }
                }
            };
            views.push(view);
        }
        Ok(views)
    }

    /// Installe `id@version` depuis `store` : archive téléchargée à l'adresse de l'index,
    /// somme vérifiée, puis importée comme une archive (source `store`).
    pub async fn install(
        &self,
        pool: &PgPool,
        context: &AuditContext,
        store_name: &str,
        id: &str,
        version: &str,
    ) -> Result<Imported, StoreError> {
        let store = self.store(store_name)?;
        let index = self.index(store, false).await?;
        let entry = index.find(id, version).ok_or(StoreError::NotInIndex)?;
        if entry.size > MAX_ARCHIVE_BYTES as u64 {
            return Err(StoreError::Untrusted(format!(
                "archive larger than {MAX_ARCHIVE_BYTES} bytes"
            )));
        }
        let url = format!("{}/{}", store.url.trim_end_matches('/'), entry.archive);
        let bytes = self
            .fetcher
            .fetch(&url, store.token.as_deref(), MAX_ARCHIVE_BYTES)
            .await
            .map_err(|e| match e {
                FetchError::NotFound => StoreError::NotInIndex,
                FetchError::Unavailable(reason) => StoreError::Unavailable(reason),
            })?;
        let digest = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        if digest != entry.sha256 {
            return Err(StoreError::Corrupted);
        }
        let loaded = ArchiveFiles::from_zip(&bytes)
            .and_then(|files| prefabs::load_package(&files, Vec::new()));
        let imported = prefabs::import_loaded(
            pool,
            context,
            loaded,
            json!({ "store": store.name, "archive_bytes": bytes.len() }),
            ImportAs::Store {
                store: store.name.clone(),
            },
        )
        .await?;
        Ok(imported)
    }
}
