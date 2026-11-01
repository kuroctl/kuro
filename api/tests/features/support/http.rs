//! Appel du routeur REST en mémoire, sans socket.

use axum::Router;
use axum::body::Body;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, SET_COOKIE};
use axum::http::{HeaderMap, Method, Request, StatusCode};
use http_body_util::BodyExt;
use kuro_api::config::AuthConfig;
use kuro_api::enrollment::http::EnrollmentSettings;
use kuro_api::iam::application::IamState;
use kuro_api::{http, state::AppState};
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;

pub struct TestResponse {
    pub status: StatusCode,
    pub content_type: Option<String>,
    pub headers: HeaderMap,
    pub json: Value,
    /// Corps brut : l'ordre des clés y est celui de la réponse (`Value` les trie).
    pub body: String,
    /// Corps tel quel (archive, image).
    pub bytes: Vec<u8>,
}

impl TestResponse {
    /// Valeur du cookie `name` posé par la réponse, s'il y en a un.
    pub fn cookie(&self, name: &str) -> Option<String> {
        self.set_cookie_header(name)?
            .split(';')
            .next()?
            .split_once('=')
            .map(|(_, value)| value.to_owned())
    }

    /// En-tête `Set-Cookie` complet du cookie `name`.
    pub fn set_cookie_header(&self, name: &str) -> Option<String> {
        self.headers
            .get_all(SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .find(|value| value.starts_with(&format!("{name}=")))
            .map(str::to_owned)
    }
}

/// Durée d'une console avant reconnexion.
pub fn console_access() -> kuro_api::servers::application::ConsoleAccess {
    kuro_api::servers::application::ConsoleAccess {
        max_session: std::time::Duration::from_secs(900),
    }
}

/// Politique HTTP des tests : `PANEL_ORIGIN` autorisée en plus de l'origine de l'API.
pub fn http_policy() -> kuro_api::http::policy::HttpPolicy {
    kuro_api::http::policy::HttpPolicy::from_config(&kuro_api::config::HttpConfig {
        allowed_origins: vec![PANEL_ORIGIN.to_owned()],
        ..kuro_api::config::HttpConfig::default()
    })
}

/// Origine du panel déclarée dans `http.allowed_origins` des tests.
pub const PANEL_ORIGIN: &str = "https://panel.example.com";

/// Requête vers les routes publiques : aucune base n'est contactée (pool paresseux).
pub async fn get(path: &str) -> TestResponse {
    get_with_panel(None, path).await
}

/// Idem, avec un panel compilé servi depuis `panel` (ADR-0016).
pub async fn get_with_panel(panel: Option<&std::path::Path>, path: &str) -> TestResponse {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://unused@127.0.0.1:1/unused")
        .unwrap();
    let iam = IamState::new(pool.clone(), &auth_config(false)).unwrap();
    let links = std::sync::Arc::new(kuro_api::ws::LinkRegistry::default());
    let servers = kuro_api::servers::application::ServersState {
        events: kuro_api::servers::application::ServerEventSink::default(),
        pool: pool.clone(),
        commands: links.clone(),
        secrets_key: *iam.keys.server_secrets(),
        console: kuro_api::servers::application::ConsoleHub::new(links.clone()),
        console_access: console_access(),
        content: kuro_api::content::ContentSources {
            modrinth: std::sync::Arc::new(super::content::FakeModrinth::default()),
            curseforge: std::sync::Arc::new(super::content::FakeSource::curseforge()),
            steam: std::sync::Arc::new(super::content::FakeSource::steam()),
            steam_store: std::sync::Arc::new(super::content::FakeSteamStore::default()),
            steam_grid: std::sync::Arc::new(super::content::FakeSteamGrid::default()),
        },
    };
    let content_sources = kuro_api::content::settings::ContentSourcesState {
        pool: pool.clone(),
        key: *iam.keys.server_secrets(),
        sources: servers.content.clone(),
    };
    let dns = kuro_api::dns::application::DnsState::new(
        pool.clone(),
        *iam.keys.server_secrets(),
        std::sync::Arc::new(super::dns::FakeDns::default()),
    );
    let secrets_key = *iam.keys.server_secrets();
    let mail = kuro_api::mail::MailState {
        pool: pool.clone(),
        key: secrets_key,
        mailer: std::sync::Arc::new(super::mail::FakeMailer::default()),
        panel_url: None,
        allow_insecure: false,
    };
    let notifications = kuro_api::notifications::application::NotificationsState {
        pool: pool.clone(),
        key: secrets_key,
        sender: std::sync::Arc::new(super::notifications::FakeWebhooks::default()),
        panel_url: None,
        allow_private_targets: false,
        mail: mail.clone(),
    };
    let setup = kuro_api::setup::SetupState {
        pool: pool.clone(),
        iam: iam.clone(),
        token_file: std::env::temp_dir().join(format!("kuro-test-setup-{}", uuid::Uuid::now_v7())),
    };
    let state = AppState {
        setup,
        mail,
        sftp: kuro_api::sftp::SftpState { pool: pool.clone() },
        client_identity: kuro_api::extension::ClientIdentity::default(),
        iam,
        enrollment: enrollment_settings(),
        links,
        servers,
        pool,
        panel: panel.map(kuro_api::http::panel::Panel::new),
        prefabs_dir: kuro_api::prefabs::application::PrefabsDir {
            dir: std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../prefabs"),
            publish_to_dir: false,
        },
        stores: kuro_api::prefabs::store::StoresState::new(
            Vec::new(),
            std::sync::Arc::new(super::store::FakeStore::default()),
        ),
        notifications,
        oauth: None,
        dns,
        content_sources,
        http: http_policy(),
        readiness: kuro_api::system::Readiness::default(),
    };
    call(&http::router(state), Method::GET, path, None, None, None).await
}

pub async fn call(
    router: &Router,
    method: Method,
    path: &str,
    token: Option<&str>,
    cookie: Option<&str>,
    body: Option<Value>,
) -> TestResponse {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        request = request.header(AUTHORIZATION, format!("Bearer {token}"));
    }
    if let Some(cookie) = cookie {
        request = request.header(COOKIE, cookie);
    }
    let request = match body {
        Some(body) => request
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string())),
        None => request.body(Body::empty()),
    }
    .unwrap();

    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let content_type = headers
        .get(CONTENT_TYPE)
        .map(|value| value.to_str().unwrap().to_owned());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

    TestResponse {
        status,
        content_type,
        headers,
        json,
        body: String::from_utf8_lossy(&bytes).into_owned(),
        bytes: bytes.to_vec(),
    }
}

/// Requête aux en-têtes libres ; `body` : type de contenu et corps encodé.
pub async fn call_raw(
    router: &Router,
    method: Method,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<(&str, String)>,
) -> TestResponse {
    let mut request = Request::builder().method(method).uri(path);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let request = match body {
        Some((content_type, body)) => request
            .header(CONTENT_TYPE, content_type)
            .body(Body::from(body)),
        None => request.body(Body::empty()),
    }
    .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let content_type = headers
        .get(CONTENT_TYPE)
        .map(|value| value.to_str().unwrap().to_owned());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    TestResponse {
        status,
        content_type,
        json: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        headers,
        body: String::from_utf8_lossy(&bytes).into_owned(),
        bytes: bytes.to_vec(),
    }
}

/// Requête au corps binaire (envoi de fichier), avec `Content-Length` si demandé.
pub async fn call_bytes(
    router: &Router,
    method: Method,
    path: &str,
    token: &str,
    body: Vec<u8>,
    with_length: bool,
) -> TestResponse {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .header(CONTENT_TYPE, "application/octet-stream");
    if with_length {
        request = request.header(axum::http::header::CONTENT_LENGTH, body.len());
    }
    let response = router
        .clone()
        .oneshot(request.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let content_type = headers
        .get(CONTENT_TYPE)
        .map(|value| value.to_str().unwrap().to_owned());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    TestResponse {
        status,
        content_type,
        headers,
        json: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        body: String::from_utf8_lossy(&bytes).into_owned(),
        bytes: bytes.to_vec(),
    }
}

/// Paramètres d'enrôlement de test (empreinte factice : aucun daemon ne s'y connecte).
pub fn enrollment_settings() -> EnrollmentSettings {
    EnrollmentSettings {
        token_ttl: std::time::Duration::from_secs(900),
        ca_fingerprint: "0".repeat(64),
        daemon_port: 8443,
    }
}

/// Configuration d'authentification de test (clé fixe).
pub fn auth_config(require_totp: bool) -> AuthConfig {
    AuthConfig {
        secret_key: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned(),
        require_totp,
        access_ttl_secs: 900,
        refresh_ttl_secs: 3600,
        session_max_age_secs: 30 * 24 * 3600,
    }
}
