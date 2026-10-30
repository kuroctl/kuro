use std::fmt;
use std::str::FromStr;

use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(Uuid);

impl NodeId {
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for NodeId {
    fn default() -> Self {
        Self::new()
    }
}

impl From<Uuid> for NodeId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl From<NodeId> for Uuid {
    fn from(id: NodeId) -> Self {
        id.0
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// `pending` : créé, pas encore enrôlé. `online` : lien ouvert et `hello` accepté.
/// `revoked` : certificat refusé, réenrôlement nécessaire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeStatus {
    Pending,
    Online,
    Offline,
    Revoked,
}

impl NodeStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Online => "online",
            Self::Offline => "offline",
            Self::Revoked => "revoked",
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unknown node status {0:?}")]
pub struct UnknownNodeStatus(pub String);

impl FromStr for NodeStatus {
    type Err = UnknownNodeStatus;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pending" => Ok(Self::Pending),
            "online" => Ok(Self::Online),
            "offline" => Ok(Self::Offline),
            "revoked" => Ok(Self::Revoked),
            other => Err(UnknownNodeStatus(other.to_owned())),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Node {
    pub id: NodeId,
    pub name: String,
    pub status: NodeStatus,
}
