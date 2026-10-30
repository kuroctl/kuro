//! Modèle du domaine `nodes`, sans I/O.

mod fingerprint;
mod host;
mod instance;
mod links;
mod name;
mod node;
mod placement;

pub use fingerprint::CertFingerprint;
pub use host::NodeHost;
pub use instance::InstanceId;
pub use links::{DisconnectReason, NoLiveLinks, NodeLinks};
pub use name::{InvalidNodeName, NodeName};
pub use node::{Node, NodeId, NodeStatus, UnknownNodeStatus};
pub use placement::{
    Coordinates, InvalidCoordinates, InvalidRegionSlug, Placement, RegionSlug, is_valid_label,
    is_valid_location,
};
