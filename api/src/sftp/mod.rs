//! Accès SFTP aux fichiers des serveurs (ADR-0047). Le SFTP est servi par chaque nœud ; l'API
//! tient les clés des comptes, décide des connexions et des baux, et journalise les sessions.
//! `domain` : nom d'utilisateur et clés publiques ; `application` : cas d'usage, y compris les
//! réponses aux requêtes `sftp.*` du lien daemon ; `http` : clés du compte et coordonnées.

pub mod application;
pub mod domain;
pub mod http;
pub mod infra;

pub use application::{SftpError, SftpState};
