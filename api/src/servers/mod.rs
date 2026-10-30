//! Domaine `servers` : serveurs déployés sur les nœuds à partir de prefabs (ADR-0013).
//!
//! L'API résout le prefab en spécification et l'envoie au nœud ; l'état du serveur suit ce que
//! le nœud rapporte. Le lien daemon (`crate::ws`) implémente les ports de `domain`.

pub mod application;
pub mod domain;
pub mod http;
pub mod infra;
