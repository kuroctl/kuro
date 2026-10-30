//! Domaine `enrollment` : ajout d'un nœud et échange token + CSR contre un certificat.
//!
//! Chaque action est un cas d'usage unique de `application`, appelé tel quel par la CLI
//! (`kuro-api node add`), l'endpoint daemon (`POST /api/enroll`) et, plus tard, le REST du panel.

pub mod application;
pub mod domain;
pub mod http;
pub mod infra;
