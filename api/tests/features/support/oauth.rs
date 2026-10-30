//! Serveur d'autorisation du MCP en test : documents de métadonnées des clients simulés.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures_util::future::BoxFuture;
use kuro_api::oauth::application::OAuthState;
use kuro_api::oauth::metadata::{FetchError, MetadataFetcher};
use sqlx::PgPool;

/// URL publique de l'API dans les tests.
pub const ISSUER: &str = "https://kuro.test";

/// Documents servis par URL ; une URL absente ne répond pas.
#[derive(Debug, Default)]
pub struct FakeFetcher {
    documents: Mutex<HashMap<String, Vec<u8>>>,
    pub fetches: Mutex<u32>,
}

impl FakeFetcher {
    pub fn serve(&self, url: &str, document: serde_json::Value) {
        self.documents
            .lock()
            .unwrap()
            .insert(url.to_owned(), document.to_string().into_bytes());
    }
}

impl MetadataFetcher for FakeFetcher {
    fn fetch(&self, url: &str) -> BoxFuture<'_, Result<Vec<u8>, FetchError>> {
        *self.fetches.lock().unwrap() += 1;
        let document = self.documents.lock().unwrap().get(url).cloned();
        Box::pin(
            async move { document.ok_or_else(|| FetchError::Unavailable("not served".to_owned())) },
        )
    }
}

pub fn oauth_state(pool: &PgPool, fetcher: Arc<FakeFetcher>) -> OAuthState {
    OAuthState::new(
        pool.clone(),
        ISSUER,
        Some("https://panel.kuro.test"),
        fetcher,
    )
    .unwrap()
}
