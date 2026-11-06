//! Nœuds à la demande (ADR-0065) : machines louées chez un hébergeur, enrôlées comme nœuds,
//! rendues, endormies et réveillées ; un passage de ménage par fonction pure.

pub mod application;
pub mod domain;
pub mod http;
pub mod provider;
pub mod providers;
