//! Journal d'audit transverse (ADR-0008, ISO 27001 A.8.15, RGPD art. 5.2 et 30).
//!
//! Toute écriture métier passe par [`infra::AuditedTx`], qui ne se valide qu'avec un événement :
//! pas de modification sans trace. Les refus d'accès et l'authentification sont tracés aussi.

pub mod application;
pub mod domain;
pub mod http;
pub mod infra;
