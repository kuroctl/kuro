/// Nom d'un rôle : identifiant lisible (`support-n1`), 64 caractères au plus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleName(String);

#[derive(Debug, thiserror::Error)]
#[error("invalid role name {0:?}: expected 1-64 chars of [a-z0-9-]")]
pub struct InvalidRoleName(pub String);

impl RoleName {
    pub fn parse(value: &str) -> Result<Self, InvalidRoleName> {
        if crate::validation::is_slug(value, 64) {
            Ok(Self(value.to_owned()))
        } else {
            Err(InvalidRoleName(value.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
