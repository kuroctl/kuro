//! Orchestrateur central : API REST du panel et point d'entrée des daemons.
//!
//! Organisation (voir ADR-0004) :
//! - transverse : `config`, `state`, `db`, `http` (REST, OpenAPI), `ws` (lien daemon mTLS) ;
//! - métier : un module par domaine (`system`, `nodes`, `enrollment`, `pki`, puis `auth`…),
//!   chacun découpé en couches `domain` / `application` / `infra` / `http` selon ses besoins.

pub mod audit;
pub mod auth;
pub mod cluster;
pub mod config;
pub mod content;
pub mod db;
pub mod dns;
pub mod enrollment;
pub mod error_code;
pub mod http;
pub mod iam;
pub mod mcp;
pub mod nodes;
pub mod notifications;
pub mod oauth;
pub mod pki;
pub mod prefabs;
pub mod servers;
pub mod settings;
pub mod setup;
pub mod state;
pub mod studio;
pub mod system;
pub mod validation;
pub mod ws;
