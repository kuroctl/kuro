//! Store de prefabs simulé (ADR-0042) : des fichiers servis par adresse, un index signé en
//! minisign (format legacy `Ed`, sans préhachage) avec une clé Ed25519 déterministe.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::{Signer, SigningKey};
use futures_util::future::BoxFuture;
use kuro_api::config::StoreConfig;
use kuro_api::prefabs::store::{FetchError, StoreFetcher};
use kuro_prefab::Package;
use kuro_prefab::assets::load_assets_from;
use kuro_prefab::files::{ArchiveFiles, zip_folder};
use kuro_prefab::package::read_folder;
use kuro_prefab::store::{INDEX_FILE, IndexEntry, StoreIndex};
use sha2::{Digest, Sha256};

/// Racine du store simulé.
pub const STORE_URL: &str = "https://store.test";
const KEY_ID: [u8; 8] = *b"kurotest";

#[derive(Debug)]
pub struct FakeStore {
    /// Adresse complète → contenu.
    pub files: Mutex<HashMap<String, Vec<u8>>>,
    /// Adresses demandées, avec le jeton reçu.
    pub requests: Mutex<Vec<(String, Option<String>)>>,
    /// Lecture en panne : tout échoue.
    pub unavailable: Mutex<bool>,
    key: SigningKey,
}

impl Default for FakeStore {
    fn default() -> Self {
        Self {
            files: Mutex::default(),
            requests: Mutex::default(),
            unavailable: Mutex::default(),
            key: SigningKey::from_bytes(&[7u8; 32]),
        }
    }
}

impl FakeStore {
    /// Configuration du store, avec sa clé publique (format minisign : `Ed`, identifiant, clé).
    pub fn config(&self) -> StoreConfig {
        StoreConfig {
            name: "test".to_owned(),
            url: format!("{STORE_URL}/"),
            public_key: self.public_key(),
            token: None,
        }
    }

    pub fn public_key(&self) -> String {
        let mut bytes = Vec::with_capacity(42);
        bytes.extend_from_slice(b"Ed");
        bytes.extend_from_slice(&KEY_ID);
        bytes.extend_from_slice(self.key.verifying_key().as_bytes());
        STANDARD.encode(bytes)
    }

    /// Archive d'un prefab officiel, manifest retouché par `edit`, déposée dans le store ; rend
    /// son entrée d'index.
    pub fn publish_folder(&self, folder: &str, edit: impl Fn(&str) -> String) -> IndexEntry {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../prefabs/{folder}"));
        let mut files = read_folder(&dir).unwrap();
        let manifest = String::from_utf8(files["prefab.yaml"].clone())
            .unwrap()
            .replace("\r\n", "\n");
        files.insert("prefab.yaml".to_owned(), edit(&manifest).into_bytes());
        let archive = ArchiveFiles::from_map(files.clone()).unwrap();
        let package = Package::load(&archive).unwrap();
        let assets = load_assets_from(&archive, &package.manifest).unwrap();
        let bytes = zip_folder(&package.manifest.id, &files).unwrap();
        let file = format!(
            "{}-{}.kprefab",
            package.manifest.id, package.manifest.version
        );
        let (path, sha256, size) = self.put_archive(&file, bytes);
        IndexEntry::describe(&package, &assets, path, sha256, size)
    }

    /// Dépose un fichier à `path` (relatif à la racine).
    pub fn put(&self, path: &str, bytes: Vec<u8>) {
        self.files
            .lock()
            .unwrap()
            .insert(format!("{STORE_URL}/{path}"), bytes);
    }

    /// Dépose une archive et rend `(chemin, sha256, taille)` pour l'index.
    pub fn put_archive(&self, file: &str, bytes: Vec<u8>) -> (String, String, u64) {
        let path = format!("archives/{file}");
        let sha256 = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let size = bytes.len() as u64;
        self.put(&path, bytes);
        (path, sha256, size)
    }

    /// Publie l'index et sa signature.
    pub fn publish(&self, index: &StoreIndex) {
        let bytes = serde_json::to_vec_pretty(index).unwrap();
        let signature = self.sign(&bytes);
        self.put(INDEX_FILE, bytes);
        self.put(&format!("{INDEX_FILE}.minisig"), signature.into_bytes());
    }

    /// Publie l'index tel quel mais signé par une autre clé.
    pub fn publish_with_other_key(&self, index: &StoreIndex) {
        let bytes = serde_json::to_vec_pretty(index).unwrap();
        let other = FakeStore {
            key: SigningKey::from_bytes(&[9u8; 32]),
            ..Default::default()
        };
        let signature = other.sign(&bytes);
        self.put(INDEX_FILE, bytes);
        self.put(&format!("{INDEX_FILE}.minisig"), signature.into_bytes());
    }

    /// Signature minisign legacy (`Ed`) : signature du fichier, puis signature globale de
    /// `signature || commentaire de confiance`.
    fn sign(&self, data: &[u8]) -> String {
        let signature = self.key.sign(data).to_bytes();
        let trusted = "kuro test store";
        let mut global = Vec::with_capacity(64 + trusted.len());
        global.extend_from_slice(&signature);
        global.extend_from_slice(trusted.as_bytes());
        let global_signature = self.key.sign(&global).to_bytes();
        let mut first = Vec::with_capacity(74);
        first.extend_from_slice(b"Ed");
        first.extend_from_slice(&KEY_ID);
        first.extend_from_slice(&signature);
        format!(
            "untrusted comment: fake store\n{}\ntrusted comment: {trusted}\n{}\n",
            STANDARD.encode(first),
            STANDARD.encode(global_signature)
        )
    }
}

impl StoreFetcher for FakeStore {
    fn fetch(
        &self,
        url: &str,
        token: Option<&str>,
        max: usize,
    ) -> BoxFuture<'_, Result<Vec<u8>, FetchError>> {
        let url = url.to_owned();
        let token = token.map(str::to_owned);
        Box::pin(async move {
            self.requests.lock().unwrap().push((url.clone(), token));
            if *self.unavailable.lock().unwrap() {
                return Err(FetchError::Unavailable("store down".to_owned()));
            }
            let bytes = self
                .files
                .lock()
                .unwrap()
                .get(&url)
                .cloned()
                .ok_or(FetchError::NotFound)?;
            if bytes.len() > max {
                return Err(FetchError::Unavailable("response too large".to_owned()));
            }
            Ok(bytes)
        })
    }
}
