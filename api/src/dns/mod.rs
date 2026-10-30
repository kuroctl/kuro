//! DNS des serveurs et des nœuds (ADR-0026) : un nom par nœud et par serveur dans une zone gérée
//! par Kuro, SRV déclarés par les prefabs, fournisseurs comme adaptateurs.

pub mod application;
pub mod cloudflare;
pub mod domain;
pub mod http;
pub mod infra;
pub mod provider;
