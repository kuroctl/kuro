//! Domaine `studio` : Prefab Studio (ADR-0011). Brouillons de prefabs partagés entre admins,
//! validés pendant la saisie, publiés en version importée par le même chemin qu'une archive
//! `.kprefab`.

pub mod application;
pub mod http;
pub mod infra;
pub mod trials;
