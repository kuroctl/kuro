//! Serveur d'autorisation OAuth 2.1 des agents IA (MCP, ADR-0023) : un admin délègue une partie
//! de ses droits à un client, qui agit ensuite avec des jetons opaques révocables.

pub mod application;
pub mod domain;
pub mod http;
pub mod infra;
pub mod metadata;
