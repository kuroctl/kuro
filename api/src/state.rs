//! État partagé injecté dans les handlers axum. Chaque domaine en extrait sa part (`FromRef`).

use std::sync::Arc;

use axum::extract::FromRef;
use sqlx::PgPool;

use crate::enrollment::http::EnrollmentSettings;
use crate::iam::application::IamState;
use crate::nodes::domain::NodeLinks;
use crate::servers::application::ServersState;

#[derive(Debug, Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub iam: IamState,
    pub enrollment: EnrollmentSettings,
    /// Sessions de lien de ce processus (registre du listener daemon).
    pub links: Arc<dyn NodeLinks>,
    pub servers: ServersState,
    /// Panel compilé servi par l'API (`http.panel_dir`), absent en développement.
    pub panel: Option<crate::http::panel::Panel>,
    /// Dossier des prefabs officiels, relu par le bouton « Recharger » du panel.
    pub prefabs_dir: crate::prefabs::application::PrefabsDir,
    /// Stores de prefabs (ADR-0042) : index signés, installation à la demande.
    pub stores: crate::prefabs::store::StoresState,
    /// Notifications et webhooks (ADR-0044).
    pub notifications: crate::notifications::application::NotificationsState,
    /// Serveur d'autorisation du MCP, si `mcp.public_url` est configurée (ADR-0023).
    pub oauth: Option<crate::oauth::application::OAuthState>,
    /// DNS des serveurs et des nœuds (ADR-0026).
    pub dns: crate::dns::application::DnsState,
    /// Clés d'API des sources de contenu (ADR-0029).
    pub content_sources: crate::content::settings::ContentSourcesState,
    /// Proxys de confiance, limitation de débit et délai de requête (`[http]`).
    pub http: crate::http::policy::HttpPolicy,
    /// Disponibilité de l'instance (`/api/ready`) : état du listener daemon.
    pub readiness: crate::system::Readiness,
}

impl FromRef<AppState> for crate::system::Readiness {
    fn from_ref(state: &AppState) -> Self {
        state.readiness.clone()
    }
}

impl FromRef<AppState> for crate::http::policy::HttpPolicy {
    fn from_ref(state: &AppState) -> Self {
        state.http.clone()
    }
}

impl FromRef<AppState> for crate::content::settings::ContentSourcesState {
    fn from_ref(state: &AppState) -> Self {
        state.content_sources.clone()
    }
}

impl FromRef<AppState> for crate::dns::application::DnsState {
    fn from_ref(state: &AppState) -> Self {
        state.dns.clone()
    }
}

impl FromRef<AppState> for Option<crate::oauth::application::OAuthState> {
    fn from_ref(state: &AppState) -> Self {
        state.oauth.clone()
    }
}

impl FromRef<AppState> for crate::prefabs::application::PrefabsDir {
    fn from_ref(state: &AppState) -> Self {
        state.prefabs_dir.clone()
    }
}

impl FromRef<AppState> for crate::prefabs::store::StoresState {
    fn from_ref(state: &AppState) -> Self {
        state.stores.clone()
    }
}

impl FromRef<AppState> for crate::notifications::application::NotificationsState {
    fn from_ref(state: &AppState) -> Self {
        state.notifications.clone()
    }
}

impl FromRef<AppState> for ServersState {
    fn from_ref(state: &AppState) -> Self {
        state.servers.clone()
    }
}

impl FromRef<AppState> for Arc<dyn NodeLinks> {
    fn from_ref(state: &AppState) -> Self {
        Arc::clone(&state.links)
    }
}

impl FromRef<AppState> for IamState {
    fn from_ref(state: &AppState) -> Self {
        state.iam.clone()
    }
}

impl FromRef<AppState> for PgPool {
    fn from_ref(state: &AppState) -> Self {
        state.pool.clone()
    }
}

impl FromRef<AppState> for EnrollmentSettings {
    fn from_ref(state: &AppState) -> Self {
        state.enrollment.clone()
    }
}
