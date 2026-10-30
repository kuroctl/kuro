//! Listener des daemons (port 8443) : TLS de la CA privée, certificat client facultatif au
//! niveau TLS mais exigé par route.
//!
//! - avec certificat client : seule la WebSocket [`DAEMON_PATH`] (lien des nœuds enrôlés) ;
//! - sans certificat client : seul `POST /api/enroll` ([`crate::enrollment`]).
//!
//! Transport uniquement : les règles de présence vivent dans [`crate::nodes`].

mod listener;
mod registry;
mod relay;
mod session;
mod tls;
mod upgrade;

use crate::nodes::domain::CertFingerprint;

pub use kuro_protocol::DAEMON_PATH;
pub use listener::{DaemonListener, LinkSetupError};
pub use registry::LinkRegistry;
pub use relay::CommandRelay;

/// Certificat client présenté sur la connexion TLS, injecté dans chaque requête.
#[derive(Debug, Clone)]
pub struct PeerCertificate(pub Option<CertFingerprint>);
