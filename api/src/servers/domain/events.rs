//! Événements du cycle de vie d'un serveur (ADR-0051 § 6), transmis aux extensions compilées
//! après la validation de leur transaction. Le propriétaire (`owner_ref`) les accompagne : une
//! extension retrouve ainsi le client concerné sans relire le serveur.

use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerEvent {
    /// Créé (`deploying`) : le déploiement commence.
    Created {
        server_id: Uuid,
        owner_ref: Option<String>,
    },
    /// Installation terminée (première ou réinstallation) : le nœud le rapporte prêt.
    Installed {
        server_id: Uuid,
        owner_ref: Option<String>,
    },
    /// Installation échouée, sur le nœud ou avant d'y arriver ; `reason` : code ou message.
    InstallFailed {
        server_id: Uuid,
        owner_ref: Option<String>,
        reason: Option<String>,
    },
    Suspended {
        server_id: Uuid,
        owner_ref: Option<String>,
    },
    Unsuspended {
        server_id: Uuid,
        owner_ref: Option<String>,
    },
    /// Suppression confirmée par le nœud : le serveur n'existe plus.
    Deleted {
        server_id: Uuid,
        owner_ref: Option<String>,
    },
    OwnerChanged {
        server_id: Uuid,
        from: Option<String>,
        to: Option<String>,
    },
}

impl ServerEvent {
    /// Nom stable (`server_installed`…), pour les journaux.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Created { .. } => "server_created",
            Self::Installed { .. } => "server_installed",
            Self::InstallFailed { .. } => "server_install_failed",
            Self::Suspended { .. } => "server_suspended",
            Self::Unsuspended { .. } => "server_unsuspended",
            Self::Deleted { .. } => "server_deleted",
            Self::OwnerChanged { .. } => "server_owner_changed",
        }
    }

    pub fn server_id(&self) -> Uuid {
        match self {
            Self::Created { server_id, .. }
            | Self::Installed { server_id, .. }
            | Self::InstallFailed { server_id, .. }
            | Self::Suspended { server_id, .. }
            | Self::Unsuspended { server_id, .. }
            | Self::Deleted { server_id, .. }
            | Self::OwnerChanged { server_id, .. } => *server_id,
        }
    }
}

/// Fin d'installation d'après la transition d'état rapportée : depuis `deploying` ou
/// `installing`, prêt (`offline`, `starting`, `running`) ou en échec (`failed`).
pub fn install_outcome(previous: &str, next: &str) -> Option<bool> {
    if !matches!(previous, "deploying" | "installing") {
        return None;
    }
    match next {
        "offline" | "starting" | "running" => Some(true),
        "failed" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_installation_ends_when_a_deploying_or_installing_server_settles() {
        assert_eq!(install_outcome("installing", "offline"), Some(true));
        assert_eq!(install_outcome("deploying", "running"), Some(true));
        assert_eq!(install_outcome("installing", "failed"), Some(false));
        assert_eq!(install_outcome("deploying", "installing"), None);
        assert_eq!(install_outcome("offline", "running"), None);
        assert_eq!(install_outcome("running", "failed"), None);
    }
}
