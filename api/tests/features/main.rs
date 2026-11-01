//! Tests de fonctionnalités de l'API : un module par feature, un seul binaire de test.
//!
//! Les features qui touchent la base démarrent un Postgres via testcontainers (Docker requis).
// Code de test : les helpers hors `#[test]` peuvent paniquer comme les tests eux-mêmes.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod admin;
mod audit;
mod client;
mod cluster;
mod config;
mod daemon_link;
mod enrollment;
mod errors;
mod extension;
mod openapi;
mod panel;
mod rate_limit;
mod server_events;
mod servers;
mod settings;
mod setup;
mod sftp;
mod support;
mod system_health;
mod testing_harness;
