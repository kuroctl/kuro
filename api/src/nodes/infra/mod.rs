//! Adaptateurs du domaine `nodes`.

mod allocations;
mod instances;
mod regions;
mod repository;

pub use allocations::{AllocationRecord, AllocationRepository};
pub use instances::{InstanceRepository, Swept};
pub use regions::{RegionRecord, RegionRepository};
pub use repository::{LockedNode, NodeRecord, NodeRepository, PlacementPatch, RegionFilter};
