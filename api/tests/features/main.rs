//! Tests de fonctionnalités de l'API : un module par feature, un seul binaire de test.
//!
//! Les features qui touchent la base démarrent un Postgres via testcontainers (Docker requis).
// Code de test : les helpers hors `#[test]` peuvent paniquer comme les tests eux-mêmes.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod admin;
mod audit;
mod cluster;
mod config;
mod daemon_link;
mod enrollment;
mod errors;
mod openapi;
mod panel;
mod rate_limit;
mod servers;
mod settings;
mod setup;
mod support;
mod system_health;
