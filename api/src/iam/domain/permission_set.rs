use std::collections::BTreeSet;

use super::Permission;

/// Permission du rôle `super-admin` : toutes, y compris celles ajoutées plus tard.
pub const WILDCARD: &str = "*";

/// Permissions effectives d'un compte ou contenu d'un rôle.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermissionSet {
    all: bool,
    granted: BTreeSet<Permission>,
}

impl PermissionSet {
    pub fn all() -> Self {
        Self {
            all: true,
            granted: BTreeSet::new(),
        }
    }

    /// Clés stockées en base. Une clé inconnue (permission retirée du code) est ignorée.
    pub fn from_keys<I, S>(keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut set = Self::default();
        for key in keys {
            let key = key.as_ref();
            if key == WILDCARD {
                set.all = true;
            } else if let Some(permission) = Permission::from_key(key) {
                set.granted.insert(permission);
            }
        }
        set
    }

    pub fn from_permissions(permissions: impl IntoIterator<Item = Permission>) -> Self {
        Self {
            all: false,
            granted: permissions.into_iter().collect(),
        }
    }

    pub fn is_all(&self) -> bool {
        self.all
    }

    pub fn contains(&self, permission: Permission) -> bool {
        self.all || self.granted.contains(&permission)
    }

    /// Anti-escalade : `self` détient tout ce que contient `other`.
    pub fn covers(&self, other: &PermissionSet) -> bool {
        self.all || (!other.all && other.granted.is_subset(&self.granted))
    }

    pub fn union(mut self, other: &PermissionSet) -> Self {
        self.all |= other.all;
        self.granted.extend(other.granted.iter().copied());
        self
    }

    /// Permissions explicites, triées (vide pour `all`).
    pub fn permissions(&self) -> impl Iterator<Item = Permission> + '_ {
        self.granted.iter().copied()
    }

    /// Permissions effectives, catalogue complet pour `all`.
    pub fn effective(&self) -> Vec<Permission> {
        if self.all {
            Permission::ALL.to_vec()
        } else {
            self.granted.iter().copied().collect()
        }
    }

    /// Forme stockée.
    pub fn keys(&self) -> Vec<&'static str> {
        if self.all {
            vec![WILDCARD]
        } else {
            self.granted
                .iter()
                .map(|permission| permission.key())
                .collect()
        }
    }
}
