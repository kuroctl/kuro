use kuro_protocol::daemon::server::ServerState as Reported;
use serde::Serialize;
use utoipa::ToSchema;

/// État d'un serveur. `deploying` et `deleting` sont posés par l'API ; les autres reflètent ce
/// que le daemon rapporte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ServerState {
    Deploying,
    /// Étapes d'installation en cours (ADR-0017).
    Installing,
    /// Sauvegarde en cours de restauration (ADR-0018).
    Restoring,
    Offline,
    Starting,
    Running,
    Stopping,
    Failed,
    Deleting,
}

#[derive(Debug, thiserror::Error)]
#[error("unknown server state {0:?}")]
pub struct UnknownServerState(pub String);

impl ServerState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Deploying => "deploying",
            Self::Installing => "installing",
            Self::Restoring => "restoring",
            Self::Offline => "offline",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Failed => "failed",
            Self::Deleting => "deleting",
        }
    }

    /// État rapporté par le daemon ; `deleted` n'est pas un état (le serveur disparaît).
    pub fn from_reported(state: Reported) -> Option<Self> {
        match state {
            Reported::Deploying => Some(Self::Deploying),
            Reported::Installing => Some(Self::Installing),
            Reported::Restoring => Some(Self::Restoring),
            Reported::Offline => Some(Self::Offline),
            Reported::Starting => Some(Self::Starting),
            Reported::Running => Some(Self::Running),
            Reported::Stopping => Some(Self::Stopping),
            Reported::Failed => Some(Self::Failed),
            Reported::Deleted => None,
        }
    }
}

impl std::str::FromStr for ServerState {
    type Err = UnknownServerState;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Ok(match text {
            "deploying" => Self::Deploying,
            "installing" => Self::Installing,
            "restoring" => Self::Restoring,
            "offline" => Self::Offline,
            "starting" => Self::Starting,
            "running" => Self::Running,
            "stopping" => Self::Stopping,
            "failed" => Self::Failed,
            "deleting" => Self::Deleting,
            other => return Err(UnknownServerState(other.to_owned())),
        })
    }
}
