//! Domaine `system` : état de l'instance (vivacité, disponibilité).
//!
//! Pas de logique métier ni de persistance : la couche `http` et l'état de disponibilité.

pub mod http;
pub mod readiness;

pub use readiness::Readiness;
