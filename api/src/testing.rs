//! Harnais de test des extensions (ADR-0051) : un Kuro complet, avec ses extensions, monté sur
//! une base fournie par l'appelant (son Postgres de test). Activé par la fonctionnalité Cargo
//! `testing` d'une dépendance de développement ; jamais dans un binaire de production.
//!
//! Les services externes (sources de contenu, DNS, e-mail, webhooks, stores) sont les vrais
//! clients, pointés vers une adresse injoignable : un test qui en dépend échoue franchement au
//! lieu d'appeler Internet. Les nœuds ne sont pas simulés.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::audit::domain::AuditContext;
use crate::config::AuthConfig;
use crate::extension::{self, ClientIdentity, ExtensionFactory};
use crate::iam::application::IamState;
use crate::iam::application::roles::{self, NewRole};
use crate::iam::application::users::{self, NewAdminUser};
use crate::iam::domain::{Authority, Permission, SUPER_ADMIN_ROLE_ID};
use crate::servers::application::{ConsoleAccess, ConsoleHub, ServerEventSink, ServersState};
use crate::state::AppState;

pub type TestError = Box<dyn std::error::Error + Send + Sync>;

/// Mot de passe de tous les comptes créés par le harnais.
pub const PASSWORD: &str = "correct horse battery staple";

/// Adresse injoignable des services externes.
const NOWHERE: &str = "https://127.0.0.1:9";

/// Réponse JSON d'une requête de test.
#[derive(Debug)]
pub struct TestResponse {
    pub status: StatusCode,
    pub json: Value,
}

/// Kuro monté pour les tests d'une extension.
pub struct TestKuro {
    pub state: AppState,
    pub router: Router,
}

impl TestKuro {
    /// Applique les migrations de Kuro sur `pool` (base vierge), prépare les extensions
    /// (migrations, permissions, actions d'audit, codes d'erreur) et monte le routeur complet.
    pub async fn start(
        pool: PgPool,
        config: BTreeMap<String, Value>,
        factories: Vec<ExtensionFactory>,
    ) -> Result<Self, TestError> {
        crate::db::migrate(&pool).await?;
        let extensions = extension::prepare(&pool, &config, factories).await?;
        let iam = IamState::new(pool.clone(), &auth_config())?;
        let links: Arc<crate::ws::LinkRegistry> = Arc::default();
        let key = *iam.keys.server_secrets();
        let content = crate::content::ContentSources {
            modrinth: Arc::new(crate::content::modrinth::Modrinth::new(NOWHERE)?),
            curseforge: Arc::new(crate::content::curseforge::CurseForge::new(NOWHERE)?),
            steam: Arc::new(crate::content::steam::SteamWorkshop::new(NOWHERE)?),
            steam_store: Arc::new(crate::content::steam_store::SteamStoreClient::new(NOWHERE)?),
            steam_grid: Arc::new(crate::content::steamgriddb::SteamGridDb::new(NOWHERE)?),
        };
        let servers = ServersState {
            pool: pool.clone(),
            commands: links.clone(),
            secrets_key: key,
            console: ConsoleHub::new(links.clone()),
            console_access: ConsoleAccess {
                max_session: std::time::Duration::from_secs(900),
            },
            content: content.clone(),
            events: ServerEventSink::of(&extensions),
        };
        let mail = crate::mail::MailState {
            pool: pool.clone(),
            key,
            mailer: Arc::new(crate::mail::LettreMailer),
            panel_url: None,
            allow_insecure: false,
        };
        let temp = std::env::temp_dir().join(format!("kuro-testing-{}", Uuid::now_v7().simple()));
        let state = AppState {
            pool: pool.clone(),
            iam: iam.clone(),
            enrollment: crate::enrollment::http::EnrollmentSettings {
                token_ttl: std::time::Duration::from_secs(900),
                ca_fingerprint: "0".repeat(64),
                daemon_port: 8443,
            },
            links,
            servers: servers.clone(),
            panel: None,
            prefabs_dir: crate::prefabs::application::PrefabsDir {
                dir: temp.join("prefabs"),
                publish_to_dir: false,
            },
            stores: crate::prefabs::store::StoresState::new(
                Vec::new(),
                Arc::new(crate::prefabs::store::HttpFetcher::new()?),
            ),
            notifications: crate::notifications::application::NotificationsState {
                pool: pool.clone(),
                key,
                sender: Arc::new(crate::notifications::sender::HttpSender::new()?),
                panel_url: None,
                allow_private_targets: false,
                mail: mail.clone(),
            },
            oauth: None,
            dns: crate::dns::application::DnsState::new(
                pool.clone(),
                key,
                Arc::new(crate::dns::cloudflare::Cloudflare::new()?),
            ),
            content_sources: crate::content::settings::ContentSourcesState {
                pool: pool.clone(),
                key,
                sources: content,
            },
            http: crate::http::policy::HttpPolicy::from_config(
                &crate::config::HttpConfig::default(),
            ),
            readiness: crate::system::Readiness::default(),
            setup: crate::setup::SetupState {
                pool: pool.clone(),
                iam,
                token_file: temp.join("setup-token"),
            },
            mail,
            sftp: crate::sftp::SftpState { pool: pool.clone() },
            client_identity: ClientIdentity::of(&extensions),
        };
        let router = crate::http::router_with(state.clone(), extensions);
        Ok(Self { state, router })
    }

    /// Compte admin dont le rôle accorde exactement `permissions` (celles de Kuro ou d'une
    /// extension), connecté et élevé : son jeton d'accès.
    pub async fn admin_token(
        &self,
        email: &str,
        permissions: &[Permission],
    ) -> Result<String, TestError> {
        let context = AuditContext::cli("testing");
        let user = self.create_admin(email, &context).await?;
        if !permissions.is_empty() {
            let role = roles::create(
                &self.state.iam,
                &Authority::Host,
                &context,
                NewRole {
                    name: format!("t{}", &Uuid::now_v7().simple().to_string()[24..]),
                    description: String::new(),
                    color: None,
                    permissions: permissions.iter().map(|p| p.key().to_owned()).collect(),
                },
            )
            .await?;
            users::set_roles(
                &self.state.iam,
                &Authority::Host,
                &context,
                user,
                &[role.id],
            )
            .await?;
        }
        self.login(email).await
    }

    /// Compte du rôle `super-admin` (toutes les permissions), connecté et élevé.
    pub async fn super_admin_token(&self, email: &str) -> Result<String, TestError> {
        let context = AuditContext::cli("testing");
        let user = self.create_admin(email, &context).await?;
        users::set_roles(
            &self.state.iam,
            &Authority::Host,
            &context,
            user,
            &[SUPER_ADMIN_ROLE_ID],
        )
        .await?;
        self.login(email).await
    }

    /// Requête JSON au routeur complet (`/api/…`), avec un jeton `Bearer` facultatif.
    pub async fn request(
        &self,
        method: Method,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> Result<TestResponse, TestError> {
        let mut request = Request::builder().method(method).uri(path);
        if let Some(token) = token {
            request = request.header(AUTHORIZATION, format!("Bearer {token}"));
        }
        let request = match body {
            Some(body) => request
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))?,
            None => request.body(Body::empty())?,
        };
        let response = self.router.clone().oneshot(request).await?;
        let status = response.status();
        let bytes = response.into_body().collect().await?.to_bytes();
        Ok(TestResponse {
            status,
            json: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        })
    }

    async fn create_admin(&self, email: &str, context: &AuditContext) -> Result<Uuid, TestError> {
        let user = users::create(
            &self.state.iam,
            &Authority::Host,
            context,
            NewAdminUser {
                email: email.to_owned(),
                display_name: "Test".to_owned(),
                password: PASSWORD.to_owned(),
            },
        )
        .await?;
        Ok(user.id)
    }

    /// Connexion par mot de passe puis élévation (les routes sensibles l'exigent, ADR-0050).
    async fn login(&self, email: &str) -> Result<String, TestError> {
        let login = self
            .request(
                Method::POST,
                "/api/admin/v1/auth/login",
                None,
                Some(json!({ "email": email, "password": PASSWORD })),
            )
            .await?;
        let token = login.json["access_token"]
            .as_str()
            .ok_or_else(|| format!("login refused: {}", login.json))?
            .to_owned();
        let elevated = self
            .request(
                Method::POST,
                "/api/admin/v1/me/elevate",
                Some(&token),
                Some(json!({ "password": PASSWORD })),
            )
            .await?;
        if elevated.status != StatusCode::OK {
            return Err(format!("elevation refused: {}", elevated.json).into());
        }
        Ok(token)
    }
}

/// Authentification des tests : clé fixe, TOTP non exigé.
fn auth_config() -> AuthConfig {
    AuthConfig {
        secret_key: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned(),
        require_totp: false,
        access_ttl_secs: 900,
        refresh_ttl_secs: 3600,
        session_max_age_secs: 30 * 24 * 3600,
    }
}
