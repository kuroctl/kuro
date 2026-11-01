//! Modèle de l'audit, sans I/O.

mod action;
mod actor;
pub mod chain;
mod event;

pub use action::{AuditAction, AuditActionDef, AuditActionError, ExtensionAuditAction};
pub use actor::{Actor, ActorKind, AgentRef, AuditContext};
pub use event::{AuditEvent, Outcome, Target};
