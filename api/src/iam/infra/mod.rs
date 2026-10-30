//! Persistance du domaine `iam`. Fonctions associées prenant une connexion : elles s'utilisent
//! dans une [`crate::audit::infra::AuditedTx`] pour les écritures.

mod roles;
mod sessions;
mod users;

pub use roles::{RoleRecord, RoleRef, RoleRepository};
pub use sessions::{SessionRecord, SessionRepository};
pub use users::{AdminUserRecord, Credentials, UserRepository};
