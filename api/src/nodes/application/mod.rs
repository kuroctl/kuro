//! Cas d'usage du domaine `nodes`. La création d'un nœud passe par
//! [`crate::enrollment::application::add_node`], qui émet aussi son token.

pub mod allocations;
mod error;
pub mod instances;
pub mod inventory;
pub mod lifecycle;
pub mod placement;
mod presence;
pub mod regions;

pub use error::NodesError;
pub use instances::Instance;
pub use placement::{NodePatch, PlacementInput};
pub use presence::{LinkStanding, Presence};
pub use regions::{NewRegion, RegionPatch};
