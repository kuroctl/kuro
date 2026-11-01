//! Harnais de l'espace admin : routeur réel en mémoire, base neuve, comptes aux permissions
//! choisies, lecture du journal d'audit.

use axum::Router;
use axum::http::{Method, StatusCode};
use kuro_api::audit::domain::AuditContext;
use kuro_api::iam::application::{IamState, NewAdminUser, NewRole, roles, users};
use kuro_api::iam::domain::{Authority, Permission, SUPER_ADMIN_ROLE_ID};
use kuro_api::{http, state::AppState};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use super::db;
use super::http::{TestResponse, auth_config, call, call_raw, enrollment_settings};

pub const PASSWORD: &str = "correct horse battery staple";
pub const REFRESH_COOKIE: &str = "__Host-kuro_admin_refresh";

pub struct TestAdmin {
    pub pool: PgPool,
    pub iam: IamState,
    router: Router,
    /// Sources de contenu simulées du routeur : Modrinth, et CurseForge (clé d'API).
    pub content: std::sync::Arc<super::content::FakeModrinth>,
    pub curseforge: std::sync::Arc<super::content::FakeSource>,
    pub steam: std::sync::Arc<super::content::FakeSource>,
    pub steam_store: std::sync::Arc<super::content::FakeSteamStore>,
    pub steam_grid: std::sync::Arc<super::content::FakeSteamGrid>,
    /// Dépendances des cas d'usage `servers`, pour piloter le moteur des tâches planifiées.
    pub servers: kuro_api::servers::application::ServersState,
    /// Documents de métadonnées des clients OAuth simulés.
    pub metadata: std::sync::Arc<super::oauth::FakeFetcher>,
    /// Fournisseur DNS simulé, et l'état du DNS pour lancer une réconciliation.
    pub dns_provider: std::sync::Arc<super::dns::FakeDns>,
    /// Store de prefabs simulé (`test`, `https://store.test`), signé par sa propre clé.
    pub store: std::sync::Arc<super::store::FakeStore>,
    /// Webhooks simulés et l'état des notifications, pour lancer une passe d'envoi.
    pub webhooks: std::sync::Arc<super::notifications::FakeWebhooks>,
    pub notifications: kuro_api::notifications::application::NotificationsState,
    /// Client SMTP simulé (ADR-0046) : les messages envoyés s'y lisent.
    pub mailer: std::sync::Arc<super::mail::FakeMailer>,
    pub dns: kuro_api::dns::application::DnsState,
    /// Première mise en route : son fichier de jeton est propre à chaque instance.
    pub setup: kuro_api::setup::SetupState,
    /// État lu par `/api/ready` : aucun listener ne tourne ici, les tests le simulent.
    pub readiness: kuro_api::system::Readiness,
}

#[derive(Debug, Clone)]
pub struct Account {
    pub id: Uuid,
    pub email: String,
}

/// Session ouverte : jeton d'accès et cookie de rafraîchissement.
#[derive(Debug, Clone)]
pub struct Session {
    pub access_token: String,
    pub refresh_cookie: String,
}

/// Événement du journal, sous une forme facile à comparer.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AuditRow {
    pub action: String,
    pub outcome: String,
    pub actor_kind: String,
    pub actor_id: Option<Uuid>,
    pub target_id: Option<String>,
    pub details: Value,
}

impl TestAdmin {
    /// TOTP non exigé (configuration de dev).
    pub async fn start() -> Self {
        Self::with_totp(false).await
    }

    pub async fn with_totp(require_totp: bool) -> Self {
        let pool = db::fresh_pool().await;
        let links: std::sync::Arc<kuro_api::ws::LinkRegistry> = std::sync::Arc::default();
        let console = kuro_api::servers::application::ConsoleHub::new(links.clone());
        Self::on(pool, links, console, require_totp)
    }

    /// Kuro avec des extensions compilées (ADR-0051), préparées sur une base neuve : leurs
    /// migrations appliquées, leurs routes montées sous `/api/<nom>`.
    pub async fn with_extensions(
        config: std::collections::BTreeMap<String, Value>,
        factories: Vec<kuro_api::extension::ExtensionFactory>,
    ) -> Self {
        let pool = db::fresh_pool().await;
        let mounted = kuro_api::extension::prepare(&pool, &config, factories)
            .await
            .unwrap();
        let links: std::sync::Arc<kuro_api::ws::LinkRegistry> = std::sync::Arc::default();
        let console = kuro_api::servers::application::ConsoleHub::new(links.clone());
        Self::on_with(pool, links, console, false, mounted)
    }

    /// Routeur REST sur une base et un registre de liens donnés (partagés avec un listener).
    pub fn on(
        pool: PgPool,
        links: std::sync::Arc<kuro_api::ws::LinkRegistry>,
        console: kuro_api::servers::application::ConsoleHub,
        require_totp: bool,
    ) -> Self {
        Self::on_with(pool, links, console, require_totp, Vec::new())
    }

    /// Routeur REST avec des extensions déjà préparées (`extension::prepare`).
    pub fn on_with(
        pool: PgPool,
        links: std::sync::Arc<kuro_api::ws::LinkRegistry>,
        console: kuro_api::servers::application::ConsoleHub,
        require_totp: bool,
        extensions: Vec<kuro_api::extension::MountedExtension>,
    ) -> Self {
        let iam = IamState::new(pool.clone(), &auth_config(require_totp)).unwrap();
        let content = std::sync::Arc::new(super::content::FakeModrinth::default());
        let curseforge = std::sync::Arc::new(super::content::FakeSource::curseforge());
        let steam = std::sync::Arc::new(super::content::FakeSource::steam());
        let steam_store = std::sync::Arc::new(super::content::FakeSteamStore::default());
        let steam_grid = std::sync::Arc::new(super::content::FakeSteamGrid::default());
        let metadata = std::sync::Arc::new(super::oauth::FakeFetcher::default());
        let dns_provider = std::sync::Arc::new(super::dns::FakeDns::default());
        let store = std::sync::Arc::new(super::store::FakeStore::default());
        let webhooks = std::sync::Arc::new(super::notifications::FakeWebhooks::default());
        let mailer = std::sync::Arc::new(super::mail::FakeMailer::default());
        let mail = kuro_api::mail::MailState {
            pool: pool.clone(),
            key: *iam.keys.server_secrets(),
            mailer: mailer.clone(),
            panel_url: Some("https://panel.example.com".to_owned()),
            allow_insecure: false,
        };
        let notifications = kuro_api::notifications::application::NotificationsState {
            pool: pool.clone(),
            key: *iam.keys.server_secrets(),
            sender: webhooks.clone(),
            panel_url: Some("https://panel.example.com".to_owned()),
            allow_private_targets: false,
            mail: mail.clone(),
        };
        let dns = kuro_api::dns::application::DnsState::new(
            pool.clone(),
            *iam.keys.server_secrets(),
            dns_provider.clone(),
        );
        let servers = kuro_api::servers::application::ServersState {
            events: kuro_api::servers::application::ServerEventSink::of(&extensions),
            pool: pool.clone(),
            commands: links.clone(),
            secrets_key: *iam.keys.server_secrets(),
            console,
            console_access: super::http::console_access(),
            content: kuro_api::content::ContentSources {
                modrinth: content.clone(),
                curseforge: curseforge.clone(),
                steam: steam.clone(),
                steam_store: steam_store.clone(),
                steam_grid: steam_grid.clone(),
            },
        };
        let content_sources = kuro_api::content::settings::ContentSourcesState {
            pool: pool.clone(),
            key: *iam.keys.server_secrets(),
            sources: servers.content.clone(),
        };
        let readiness = kuro_api::system::Readiness::default();
        let setup = kuro_api::setup::SetupState {
            pool: pool.clone(),
            iam: iam.clone(),
            token_file: std::env::temp_dir()
                .join(format!("kuro-test-setup-{}", uuid::Uuid::now_v7())),
        };
        let router = http::router_with(
            AppState {
                pool: pool.clone(),
                iam: iam.clone(),
                enrollment: enrollment_settings(),
                links,
                servers: servers.clone(),
                panel: None,
                prefabs_dir: kuro_api::prefabs::application::PrefabsDir {
                    dir: std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../prefabs"),
                    publish_to_dir: false,
                },
                stores: kuro_api::prefabs::store::StoresState::new(
                    vec![store.config()],
                    store.clone(),
                ),
                notifications: notifications.clone(),
                oauth: Some(super::oauth::oauth_state(&pool, metadata.clone())),
                dns: dns.clone(),
                content_sources,
                http: super::http::http_policy(),
                readiness: readiness.clone(),
                setup: setup.clone(),
                mail: mail.clone(),
                sftp: kuro_api::sftp::SftpState { pool: pool.clone() },
                client_identity: kuro_api::extension::ClientIdentity::of(&extensions),
            },
            extensions,
        );
        Self {
            pool,
            iam,
            router,
            content,
            curseforge,
            steam,
            steam_store,
            steam_grid,
            servers,
            metadata,
            dns_provider,
            store,
            webhooks,
            notifications,
            mailer,
            dns,
            setup,
            readiness,
        }
    }

    /// Routeur REST, à servir sur un vrai port (WebSocket).
    pub fn router(&self) -> Router {
        self.router.clone()
    }

    pub async fn call(
        &self,
        method: Method,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> TestResponse {
        call(
            &self.router,
            method,
            &format!("/api/admin/v1{path}"),
            token,
            None,
            body,
        )
        .await
    }

    pub async fn call_with_cookie(&self, method: Method, path: &str, cookie: &str) -> TestResponse {
        let cookie = format!("{REFRESH_COOKIE}={cookie}");
        call(
            &self.router,
            method,
            &format!("/api/admin/v1{path}"),
            None,
            Some(&cookie),
            None,
        )
        .await
    }

    /// Requête aux en-têtes libres (`Origin`, `Cookie`…), corps JSON facultatif.
    pub async fn call_with_headers(
        &self,
        method: Method,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<Value>,
    ) -> TestResponse {
        call_raw(
            &self.router,
            method,
            &format!("/api/admin/v1{path}"),
            headers,
            body.map(|body| ("application/json", body.to_string())),
        )
        .await
    }

    /// Compte dont le rôle accorde exactement `permissions` (aucun rôle si vide).
    pub async fn account(&self, email: &str, permissions: &[Permission]) -> Account {
        let context = AuditContext::cli("test");
        let user = users::create(
            &self.iam,
            &Authority::Host,
            &context,
            NewAdminUser {
                email: email.to_owned(),
                display_name: "Test".to_owned(),
                password: PASSWORD.to_owned(),
            },
        )
        .await
        .unwrap();
        if !permissions.is_empty() {
            let role = roles::create(
                &self.iam,
                &Authority::Host,
                &context,
                NewRole {
                    name: format!("r{}", &Uuid::now_v7().simple().to_string()[24..]),
                    description: String::new(),
                    color: None,
                    permissions: permissions.iter().map(|p| p.key().to_owned()).collect(),
                },
            )
            .await
            .unwrap();
            users::set_roles(&self.iam, &Authority::Host, &context, user.id, &[role.id])
                .await
                .unwrap();
        }
        Account {
            id: user.id,
            email: user.email,
        }
    }

    pub async fn super_admin(&self, email: &str) -> Account {
        let account = self.account(email, &[]).await;
        users::set_roles(
            &self.iam,
            &Authority::Host,
            &AuditContext::cli("test"),
            account.id,
            &[SUPER_ADMIN_ROLE_ID],
        )
        .await
        .unwrap();
        account
    }

    /// Envoi d'un corps binaire (fichier).
    pub async fn call_bytes(
        &self,
        method: Method,
        path: &str,
        token: &str,
        body: Vec<u8>,
        with_length: bool,
    ) -> TestResponse {
        super::http::call_bytes(
            &self.router,
            method,
            &format!("/api/admin/v1{path}"),
            token,
            body,
            with_length,
        )
        .await
    }

    /// Connexion par mot de passe (TOTP non exigé), session élevée (ADR-0050) : les actions
    /// protégées passent, comme juste après une confirmation d'identité. Les tests de
    /// l'élévation partent de [`Self::login_unelevated`].
    pub async fn login(&self, account: &Account) -> Session {
        let session = self.login_unelevated(account).await;
        let elevated = self
            .call(
                Method::POST,
                "/me/elevate",
                Some(&session.access_token),
                Some(json!({ "password": PASSWORD })),
            )
            .await;
        assert_eq!(elevated.status, StatusCode::OK, "{}", elevated.json);
        session
    }

    /// Connexion par mot de passe (TOTP non exigé), sans élévation.
    pub async fn login_unelevated(&self, account: &Account) -> Session {
        let response = self.login_with(&account.email, PASSWORD).await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.json);
        assert_eq!(
            response.json["status"], "authenticated",
            "{}",
            response.json
        );
        Session {
            access_token: response.json["access_token"].as_str().unwrap().to_owned(),
            refresh_cookie: response.cookie(REFRESH_COOKIE).unwrap(),
        }
    }

    pub async fn login_with(&self, email: &str, password: &str) -> TestResponse {
        self.call(
            Method::POST,
            "/auth/login",
            None,
            Some(json!({ "email": email, "password": password })),
        )
        .await
    }

    /// Journal complet, dans l'ordre chronologique.
    pub async fn audit(&self) -> Vec<AuditRow> {
        sqlx::query_as(
            "SELECT action, outcome, actor_kind, actor_id, target_id, details
             FROM audit_log ORDER BY seq",
        )
        .fetch_all(&self.pool)
        .await
        .unwrap()
    }

    /// Fait sortir les jetons déjà remplacés de la fenêtre de grâce (`ROTATION_GRACE`).
    pub async fn expire_rotation_grace(&self) {
        sqlx::query(
            "UPDATE admin_sessions SET rotated_at = rotated_at - interval '1 minute'
             WHERE rotated_at IS NOT NULL",
        )
        .execute(&self.pool)
        .await
        .unwrap();
    }

    /// Requête hors de l'espace admin (`/oauth/…`, `/.well-known/…`, `/mcp`), corps déjà encodé.
    pub async fn raw(
        &self,
        method: Method,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<(&str, String)>,
    ) -> TestResponse {
        super::http::call_raw(&self.router, method, path, headers, body).await
    }

    /// Dernier événement portant ce code d'action.
    pub async fn last_audit(&self, action: &str) -> AuditRow {
        self.audit()
            .await
            .into_iter()
            .rev()
            .find(|row| row.action == action)
            .unwrap_or_else(|| panic!("no audit event {action}"))
    }
}
