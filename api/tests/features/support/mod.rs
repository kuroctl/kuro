//! Outillage commun des tests de fonctionnalités.

pub mod admin;
pub mod browser;
pub mod content;
pub mod db;
pub mod dns;
pub mod http;
pub mod link;
pub mod mail;
pub mod notifications;
pub mod oauth;
pub mod pki;
pub mod platform;
pub mod store;

/// Permission d'une extension (ADR-0051) : le registre est partagé par tout le binaire de test.
pub fn is_extension_permission(key: &str) -> bool {
    matches!(
        kuro_api::iam::domain::Permission::from_key(key),
        Some(kuro_api::iam::domain::Permission::Extension(_))
    )
}
