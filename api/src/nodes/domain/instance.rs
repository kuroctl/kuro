//! Identité d'une instance de l'API (ADR-0037) : tirée au démarrage de `kuro-api serve`,
//! inscrite en base, portée par les nœuds dont cette instance tient la session.

use std::fmt;

use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InstanceId(Uuid);

impl InstanceId {
    pub fn generate() -> Self {
        Self(Uuid::now_v7())
    }

    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl From<Uuid> for InstanceId {
    fn from(id: Uuid) -> Self {
        Self(id)
    }
}

impl fmt::Display for InstanceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
