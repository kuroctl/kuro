//! Implémentations du trait `Provider`, une par hébergeur (ADR-0065).

pub mod hetzner;
pub mod memory;

/// Fournisseurs connus, tels que les régions les nomment.
pub const KNOWN: &[&str] = &["hetzner"];
