//! Domaine `servers` : état d'un serveur et ports vers le lien daemon.

mod events;
mod owner;
mod ports;
pub mod schedule;
mod state;

pub use events::{ServerEvent, install_outcome};
pub use owner::{OWNER_REF_MAX, OwnerRef, is_owner_ref};
pub use ports::{CommandFailure, NodeCommands, ServerEvents};
pub use state::{ServerState, UnknownServerState};
