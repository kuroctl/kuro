use uuid::Uuid;

use super::{Permission, PermissionSet};

/// Admin authentifié par son jeton d'accès, permissions rechargées à chaque requête.
#[derive(Debug, Clone)]
pub struct Principal {
    pub user_id: Uuid,
    pub session_id: Uuid,
    pub permissions: PermissionSet,
}

/// Au nom de qui un cas d'usage s'exécute.
#[derive(Debug, Clone)]
pub enum Authority {
    /// Requête REST : soumise aux permissions et aux règles anti-escalade.
    Admin(Principal),
    /// Commande lancée sur l'hôte de l'API : l'accès au shell vaut tous les droits.
    Host,
}

impl Authority {
    pub fn has(&self, permission: Permission) -> bool {
        match self {
            Self::Admin(principal) => principal.permissions.contains(permission),
            Self::Host => true,
        }
    }

    /// Anti-escalade : on n'accorde, ne retire ni ne gère que ce qu'on détient soi-même.
    pub fn covers(&self, permissions: &PermissionSet) -> bool {
        match self {
            Self::Admin(principal) => principal.permissions.covers(permissions),
            Self::Host => true,
        }
    }

    /// Le compte agissant, s'il s'agit d'un admin.
    pub fn user_id(&self) -> Option<Uuid> {
        match self {
            Self::Admin(principal) => Some(principal.user_id),
            Self::Host => None,
        }
    }

    pub fn is_user(&self, id: Uuid) -> bool {
        self.user_id() == Some(id)
    }
}
