//! Persistance du domaine `iam`. Fonctions associées prenant une connexion : elles s'utilisent
//! dans une [`crate::audit::infra::AuditedTx`] pour les écritures.

mod elevations;
mod invitations;
mod password_resets;
mod recovery_codes;
mod roles;
mod sessions;
mod users;

pub use elevations::ElevationRepository;
pub use invitations::{InvitationRecord, InvitationRepository};
pub use password_resets::{PasswordResetRecord, PasswordResetRepository};
pub use recovery_codes::RecoveryCodeRepository;
pub use roles::{RoleRecord, RoleRef, RoleRepository};
pub use sessions::{SessionRecord, SessionRepository};
pub use users::{AdminUserRecord, Credentials, UserRepository};
