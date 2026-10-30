//! Contrat de messages API ↔ daemon.
//!
//! Enveloppe `{ "id", "reply_to"?, "type", "payload" }` transportée sur WebSocket mTLS
//! (voir ADR-0001). Un enum par sens de circulation : [`DaemonMessage`] et [`ApiMessage`].
//! Encodage et limite de taille : [`encode`] / [`decode`].
//!
//! L'identité du nœud ne figure jamais dans un payload : l'API la déduit du certificat client.

pub mod api;
pub mod codec;
pub mod daemon;
pub mod enroll;
pub mod envelope;
pub mod limits;
pub mod version;

pub use api::ApiMessage;
pub use codec::{CodecError, decode, encode};
pub use daemon::DaemonMessage;
pub use envelope::{Envelope, MessageId};
pub use version::PROTOCOL_VERSION;

/// Chemin HTTP de la poignée de main WebSocket du lien daemon.
pub const DAEMON_PATH: &str = "/api/daemon";
