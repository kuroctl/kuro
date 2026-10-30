//! Domaine `nodes` : hôtes exécutant un daemon, leur identité, leur présence et leur
//! emplacement (régions). La création et l'enrôlement relèvent de [`crate::enrollment`].

pub mod application;
pub mod domain;
pub mod http;
pub mod infra;
