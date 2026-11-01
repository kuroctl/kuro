//! Agent Kuro : pilote Docker sur l'hôte et maintient le lien sortant vers l'API.
//!
//! Deux binaires partagent cette bibliothèque :
//! - `kuro-daemon` : le service long (systemd) ;
//! - `kuro-daemonctl` : la CLI de maintenance pour l'exploitation.
//!
//! `enroll` obtient le certificat du nœud, `link` maintient la connexion à l'API, `server`
//! exécute ses commandes avec l'adaptateur `docker` et les fichiers de `fs` (ADR-0013).
//! `sftp` sert les fichiers en SFTP par un processus séparé (ADR-0047). `stats` arrive avec
//! sa tranche (ADR-0004).

pub mod backup;
pub mod config;
pub mod docker;
pub mod enroll;
pub mod fs;
pub mod install;
pub mod link;
pub mod server;
pub mod sftp;
pub mod stats;
pub mod web;
