//! Notifications et webhooks (ADR-0044) : événements émis par les cas d'usage (crash, tâche ou
//! sauvegarde en échec, mise à jour en échec, disque presque plein, nœud hors ligne ou de
//! retour), canaux (webhook signé, Discord, Slack), règles par portée, journal des envois.

pub mod application;
pub mod domain;
pub mod http;
pub mod infra;
pub mod payload;
pub mod sender;
pub mod subscriptions;
