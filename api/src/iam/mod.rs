//! Domaine `iam` : comptes de l'espace admin, rôles et permissions (RBAC), sessions (ADR-0007).
//!
//! Chaque action est un cas d'usage de `application`, appelé à l'identique par le REST et par
//! la CLI d'administration, avec une [`domain::Authority`] différente.

pub mod application;
pub mod domain;
pub mod http;
pub mod infra;
