/// Couleur du badge d'un rôle dans le panel : `#rrggbb`, normalisée en minuscules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleColor(String);

#[derive(Debug, thiserror::Error)]
#[error("invalid role color {0:?}: expected #rrggbb")]
pub struct InvalidRoleColor(pub String);

impl RoleColor {
    pub fn parse(value: &str) -> Result<Self, InvalidRoleColor> {
        let color = value.trim().to_ascii_lowercase();
        let valid = color.len() == 7
            && color.starts_with('#')
            && color[1..].bytes().all(|b| b.is_ascii_hexdigit());
        if valid {
            Ok(Self(color))
        } else {
            Err(InvalidRoleColor(value.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for RoleColor {
    /// Ardoise neutre, identique au défaut de la base.
    fn default() -> Self {
        Self("#64748b".to_owned())
    }
}
