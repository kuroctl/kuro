use std::fmt;

const MAX_LEN: usize = 63;

/// Nom d'un nœud : 1 à 63 caractères `[a-z0-9-]`, sans tiret en bordure (forme d'un label DNS).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeName(String);

#[derive(Debug, thiserror::Error)]
#[error(
    "invalid node name {0:?}: expected 1-63 chars of [a-z0-9-], not starting or ending with '-'"
)]
pub struct InvalidNodeName(pub String);

impl NodeName {
    pub fn parse(value: &str) -> Result<Self, InvalidNodeName> {
        if crate::validation::is_slug(value, MAX_LEN) {
            Ok(Self(value.to_owned()))
        } else {
            Err(InvalidNodeName(value.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NodeName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
