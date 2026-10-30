//! Domaine `prefabs` : catalogue des modèles de serveurs (ADR-0011).
//!
//! Les prefabs officiels sont importés au démarrage ; `id` + `version` est immuable. L'admin
//! (`prefabs.manage`) importe une archive `.kprefab` ou publie un brouillon du Studio (`studio`).

pub mod application;
pub mod export;
pub mod http;
pub mod infra;
