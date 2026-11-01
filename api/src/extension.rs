//! Extensions compilées avec Kuro (ADR-0051).
//!
//! Un binaire qui embarque Kuro (`kuro_api::app::main`) lui passe ses extensions. Chacune a un
//! nom, qui fixe :
//! - ses routes, montées sous `/api/<nom>/…` ;
//! - ses tables, dans le schéma Postgres `<nom>`, avec sa propre table de migrations
//!   (`<nom>._sqlx_migrations`) : aucune collision avec les versions de Kuro ;
//! - sa configuration, la section `[extensions.<nom>]` du fichier (ou `KURO_API_EXTENSIONS__…`) ;
//! - ses permissions, préfixées par son nom (`<nom>.catalog.manage`), ajoutées au catalogue et
//!   attribuables par les rôles de Kuro.
//!
//! Sans extension, Kuro se comporte exactement comme avant.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use axum::Router;
use futures_util::future::BoxFuture;
use sqlx::PgPool;
use sqlx::migrate::{MigrateError, Migrator};

use crate::audit::domain::{AuditAction, AuditActionDef, AuditActionError};
use crate::error_code::{ErrorCode, ErrorCodeDef, ErrorCodeError};
use crate::iam::domain::{Permission, PermissionDef, PermissionError};
use crate::servers::domain::{OwnerRef, ServerEvent};
use crate::state::AppState;

/// Noms refusés : routes de Kuro sous `/api/…`, schémas de Postgres, et préfixes des
/// permissions de Kuro (une extension préfixe les siennes par son nom).
const RESERVED: &[&str] = &[
    "admin",
    "client",
    "health",
    "ready",
    "setup",
    "openapi",
    "public",
    "information_schema",
    "iam",
    "audit",
    "nodes",
    "servers",
    "prefabs",
    "customers",
    "system",
];

/// Ce qu'une extension reçoit pour se construire.
pub struct ExtensionContext<'a> {
    pub pool: &'a PgPool,
    /// Section `[extensions.<nom>]` de la configuration, si elle existe.
    pub config: Option<&'a serde_json::Value>,
}

/// Fabrique d'une extension : son nom, puis sa construction une fois la base et la
/// configuration connues.
pub struct ExtensionFactory {
    name: &'static str,
    permissions: &'static [PermissionDef],
    audit_actions: &'static [AuditActionDef],
    error_codes: &'static [ErrorCodeDef],
    build: Box<BuildFn>,
}

type BuildFn = dyn Fn(&ExtensionContext<'_>) -> Result<Extension, ExtensionError> + Send + Sync;

impl ExtensionFactory {
    pub fn new(
        name: &'static str,
        build: impl Fn(&ExtensionContext<'_>) -> Result<Extension, ExtensionError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            name,
            permissions: &[],
            audit_actions: &[],
            error_codes: &[],
            build: Box::new(build),
        }
    }

    /// Permissions de l'extension (`static` : elles sont connues avant toute base, pour le
    /// document OpenAPI). Utilisées dans ses gardes par `Permission::of(&DEF)`.
    pub fn permissions(mut self, permissions: &'static [PermissionDef]) -> Self {
        self.permissions = permissions;
        self
    }

    /// Actions d'audit de l'extension (`cloud.catalog.product_created`), pour
    /// `AuditEvent::success(AuditAction::of(&DEF))` dans ses transactions auditées.
    pub fn audit_actions(mut self, actions: &'static [AuditActionDef]) -> Self {
        self.audit_actions = actions;
        self
    }

    /// Codes d'erreur de l'extension (`cloud.catalog.slug_taken`), pour
    /// `Problem::new(status, ErrorCode::of(&DEF))` ; publiés dans l'OpenAPI.
    pub fn error_codes(mut self, codes: &'static [ErrorCodeDef]) -> Self {
        self.error_codes = codes;
        self
    }

    pub fn name(&self) -> &'static str {
        self.name
    }
}

/// Vérifie les noms des extensions et enregistre leurs permissions dans le catalogue. Appelé
/// avant toute commande (le document OpenAPI les publie) et par [`prepare`].
pub fn register(factories: &[ExtensionFactory]) -> Result<(), ExtensionError> {
    let mut seen = HashSet::new();
    for factory in factories {
        check_name(factory.name)?;
        if !seen.insert(factory.name) {
            return Err(ExtensionError::Duplicate(factory.name.to_owned()));
        }
    }
    for factory in factories {
        Permission::register(factory.name, factory.permissions)?;
        AuditAction::register(factory.name, factory.audit_actions)?;
        ErrorCode::register(factory.name, factory.error_codes)?;
    }
    Ok(())
}

/// Fournisseur d'identité de l'espace client (ADR-0051 § 3) : Kuro ne stocke ni compte, ni mot
/// de passe, ni session client ; il demande à l'extension à qui correspond un jeton.
pub trait IdentityProvider: Send + Sync {
    /// Jeton `Authorization: Bearer` d'une requête de l'espace client → propriétaire ; `None`
    /// pour un jeton inconnu, expiré ou révoqué (`401`). Une erreur signale un fournisseur
    /// indisponible (`500`), jamais un jeton refusé.
    fn authenticate<'a>(
        &'a self,
        token: &'a str,
    ) -> BoxFuture<'a, Result<Option<OwnerRef>, IdentityError>>;
}

#[derive(Debug, thiserror::Error)]
#[error("identity provider unavailable: {0}")]
pub struct IdentityError(pub String);

/// Événements du cycle de vie des serveurs (ADR-0051 § 6), reçus après la validation de leur
/// transaction, dans une tâche à part : une erreur est journalisée et n'annule jamais l'action
/// de Kuro. Au plus une fois : un arrêt du processus entre l'action et l'appel perd l'événement,
/// l'extension se resynchronise en relisant les serveurs.
pub trait ServerHooks: Send + Sync {
    fn on_server_event<'a>(
        &'a self,
        event: &'a ServerEvent,
    ) -> BoxFuture<'a, Result<(), HookError>>;
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct HookError(pub String);

/// Fournisseur d'identité de l'instance, s'il y en a un : sans lui, l'espace client
/// (`/api/client/v1`) n'est pas monté.
#[derive(Clone, Default)]
pub struct ClientIdentity(Option<Arc<dyn IdentityProvider>>);

impl ClientIdentity {
    pub fn new(provider: Option<Arc<dyn IdentityProvider>>) -> Self {
        Self(provider)
    }

    /// Celui des extensions préparées ([`prepare`] garantit qu'il y en a au plus un).
    pub fn of(extensions: &[MountedExtension]) -> Self {
        Self(extensions.iter().find_map(|e| e.identity.clone()))
    }

    pub fn provider(&self) -> Option<&dyn IdentityProvider> {
        self.0.as_deref()
    }
}

impl std::fmt::Debug for ClientIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ClientIdentity")
            .field(&self.0.as_ref().map(|_| "provider"))
            .finish()
    }
}

/// Une extension construite : ses routes, ses migrations, son fournisseur d'identité.
#[derive(Default)]
pub struct Extension {
    routes: Router<AppState>,
    migrator: Option<&'static Migrator>,
    identity: Option<Arc<dyn IdentityProvider>>,
    hooks: Option<Arc<dyn ServerHooks>>,
}

impl Extension {
    /// Routes montées sous `/api/<nom>`, avec l'état de Kuro (base, IAM, serveurs…).
    pub fn routes(mut self, routes: Router<AppState>) -> Self {
        self.routes = routes;
        self
    }

    /// Migrations de l'extension (`sqlx::migrate!()`), appliquées dans son schéma. Leurs requêtes
    /// peuvent créer des tables sans schéma (`create table products`) : elles vont dans le schéma
    /// de l'extension ; les tables de Kuro restent visibles (`public` suit dans le chemin).
    pub fn migrations(mut self, migrator: &'static Migrator) -> Self {
        self.migrator = Some(migrator);
        self
    }

    /// Fournit l'identité de l'espace client. Une seule extension par binaire peut le faire.
    pub fn identity(mut self, provider: Arc<dyn IdentityProvider>) -> Self {
        self.identity = Some(provider);
        self
    }

    /// Reçoit les événements du cycle de vie des serveurs.
    pub fn hooks(mut self, hooks: Arc<dyn ServerHooks>) -> Self {
        self.hooks = Some(hooks);
        self
    }
}

/// Extension prête : migrée, ses routes à monter.
pub struct MountedExtension {
    pub name: &'static str,
    pub routes: Router<AppState>,
    pub identity: Option<Arc<dyn IdentityProvider>>,
    pub hooks: Option<Arc<dyn ServerHooks>>,
}

#[derive(Debug, thiserror::Error)]
pub enum ExtensionError {
    #[error(
        "invalid extension name {0:?}: lowercase letters, digits and `_`, starting with a letter"
    )]
    InvalidName(String),
    #[error("extension name {0:?} is reserved")]
    ReservedName(String),
    #[error("extension {0:?} is registered twice")]
    Duplicate(String),
    #[error("extensions {first:?} and {second:?} both provide the client identity")]
    IdentityTwice { first: String, second: String },
    #[error(transparent)]
    Permission(#[from] PermissionError),
    #[error(transparent)]
    AuditAction(#[from] AuditActionError),
    #[error(transparent)]
    ErrorCode(#[from] ErrorCodeError),
    #[error("invalid configuration for extension {name:?}: {reason}")]
    Config { name: String, reason: String },
    #[error("migrating extension {name:?}")]
    Migrate {
        name: String,
        #[source]
        source: MigrateError,
    },
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// Vérifie les noms, construit chaque extension et applique ses migrations, dans l'ordre reçu.
/// À appeler après les migrations de Kuro (les tables d'une extension peuvent viser les siennes).
pub async fn prepare(
    pool: &PgPool,
    config: &BTreeMap<String, serde_json::Value>,
    factories: Vec<ExtensionFactory>,
) -> Result<Vec<MountedExtension>, ExtensionError> {
    register(&factories)?;
    let mut mounted: Vec<MountedExtension> = Vec::with_capacity(factories.len());
    for factory in factories {
        let context = ExtensionContext {
            pool,
            config: config.get(factory.name),
        };
        let extension = (factory.build)(&context)?;
        if extension.identity.is_some()
            && let Some(first) = mounted.iter().find(|m| m.identity.is_some())
        {
            return Err(ExtensionError::IdentityTwice {
                first: first.name.to_owned(),
                second: factory.name.to_owned(),
            });
        }
        if let Some(migrator) = extension.migrator {
            migrate(pool, factory.name, migrator).await?;
        }
        tracing::info!(extension = factory.name, "extension loaded");
        mounted.push(MountedExtension {
            name: factory.name,
            routes: extension.routes,
            identity: extension.identity,
            hooks: extension.hooks,
        });
    }
    Ok(mounted)
}

/// Nom d'extension : `[a-z][a-z0-9_]{0,31}`, hors noms réservés et préfixe `pg_`. Il sert de
/// segment d'URL et de nom de schéma.
pub fn check_name(name: &str) -> Result<(), ExtensionError> {
    let valid = name.len() <= 32
        && name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !valid {
        return Err(ExtensionError::InvalidName(name.to_owned()));
    }
    if RESERVED.contains(&name) || name.starts_with("pg_") {
        return Err(ExtensionError::ReservedName(name.to_owned()));
    }
    Ok(())
}

/// Applique les migrations d'une extension dans son schéma, sur une connexion retirée du pool :
/// le `search_path` posé pour l'occasion ne doit pas survivre dans une connexion partagée.
async fn migrate(pool: &PgPool, name: &str, source: &Migrator) -> Result<(), ExtensionError> {
    let mut migrator = Migrator::with_migrations(source.iter().cloned().collect());
    migrator.dangerous_set_table_name(format!("{name}._sqlx_migrations"));
    migrator.create_schema(name.to_owned());

    let mut conn = pool.acquire().await?.detach();
    sqlx::query("select set_config('search_path', $1, false)")
        .bind(format!("{name}, public"))
        .execute(&mut conn)
        .await?;
    let result = migrator
        .run(&mut conn)
        .await
        .map_err(|source| ExtensionError::Migrate {
            name: name.to_owned(),
            source,
        });
    // Connexion fermée dans tous les cas : elle ne revient jamais au pool.
    let _ = sqlx::Connection::close(conn).await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_simple_names() {
        for name in ["cloud", "billing_v2", "a"] {
            assert!(check_name(name).is_ok(), "{name}");
        }
    }

    #[test]
    fn refuses_names_unfit_for_a_path_or_a_schema() {
        for name in [
            "",
            "Cloud",
            "2cloud",
            "cloud-x",
            "cloud.x",
            "../x",
            "x y",
            &"a".repeat(33),
        ] {
            assert!(
                matches!(check_name(name), Err(ExtensionError::InvalidName(_))),
                "{name}"
            );
        }
    }

    #[test]
    fn refuses_reserved_names() {
        for name in ["admin", "client", "public", "pg_catalog", "health"] {
            assert!(
                matches!(check_name(name), Err(ExtensionError::ReservedName(_))),
                "{name}"
            );
        }
    }
}
