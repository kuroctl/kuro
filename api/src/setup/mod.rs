//! Première mise en route (ADR-0045) : tant qu'aucun compte admin n'existe, l'API est « à
//! configurer ». Le premier compte, super admin, se crée par `POST /api/setup` avec le jeton de
//! mise en route écrit sur l'hôte, ou par `kuro-api setup` sur l'hôte lui-même. Le nom du projet
//! se règle au même moment.

pub mod application;
pub mod http;

pub use application::{SetupError, SetupRequest, SetupState, SetupStatus};
