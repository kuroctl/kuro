//! Socle commun aux binaires (`kuro-api`, `kuro-daemon`, `kuro-daemonctl`) : logs, arrêt propre,
//! TLS, écriture de fichiers sensibles.

pub mod fs;
pub mod shutdown;
pub mod telemetry;
pub mod tls;
