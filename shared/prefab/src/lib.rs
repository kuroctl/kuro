//! Prefabs Kuro (ADR-0011) : modèle de `prefab.yaml`, validation, gabarits, réglages et
//! résolution en spécification de déploiement (`server.deploy`, ADR-0013).
//!
//! Utilisée par l'API (import, réglages, déploiement) et, plus tard, par le Prefab Studio.

pub mod assets;
pub mod compose;
pub mod egg;
pub mod files;
pub mod manifest;
#[cfg(feature = "media")]
pub mod media;
pub mod package;
pub mod resolve;
pub mod settings;
pub mod store;
pub mod template;
pub mod validate;

pub use manifest::Manifest;
pub use package::{LoadError, Package};
pub use resolve::{
    ActionError, BackupPlan, ContentPlan, ContentSource, Deployment, ResolveError, ResolvedAction,
    SteamAccountRequired, resolve, resolve_action, resolve_backup, resolve_content,
    resolve_install, with_steam_login,
};
pub use validate::Issue;
