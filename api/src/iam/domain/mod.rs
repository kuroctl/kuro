//! Modèle du domaine `iam`, sans I/O.

mod authority;
mod email;
mod permission;
mod permission_set;
mod role_color;
mod role_name;

pub use authority::{Authority, Principal};
pub use email::{Email, InvalidEmail};
pub use permission::{Availability, Category, Permission};
pub use permission_set::{PermissionSet, WILDCARD};
pub use role_color::{InvalidRoleColor, RoleColor};
pub use role_name::{InvalidRoleName, RoleName};

/// Rôle système livré par la migration 0003 : toutes les permissions, non modifiable.
pub const SUPER_ADMIN_ROLE_ID: uuid::Uuid = uuid::uuid!("00000000-0000-7000-8000-000000000001");

/// Libellé d'un compte : 1 à 100 caractères affichables.
pub fn is_valid_display_name(value: &str) -> bool {
    crate::validation::is_display_text(value.trim(), 1, 100)
}

/// Description d'un rôle : 500 caractères au plus.
pub fn is_valid_description(value: &str) -> bool {
    crate::validation::is_display_text(value, 0, 500)
}
