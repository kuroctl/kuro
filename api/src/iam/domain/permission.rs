//! Catalogue des permissions de l'espace admin : **source de vérité** (ADR-0007).
//!
//! Format `domaine.ressource.action`. Une permission `Reserved` existe pour figer son nom et son
//! périmètre, mais n'est pas attribuable tant qu'aucune route ne l'utilise.
//! Une permission sensible (`sensitive = true`) est mise en évidence dans le panel.
//!
//! Une extension compilée avec Kuro (ADR-0051) ajoute les siennes, préfixées par son nom
//! (`cloud.catalog.manage`) : [`Permission::Extension`], enregistrées au démarrage.

use std::collections::BTreeMap;
use std::sync::RwLock;

use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Iam,
    Audit,
    Nodes,
    Servers,
    Prefabs,
    Customers,
    System,
    /// Permissions des extensions compilées (ADR-0051).
    Extensions,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Self::Iam => "Accès et sécurité",
            Self::Audit => "Journal d'audit",
            Self::Nodes => "Infrastructure",
            Self::Servers => "Serveurs",
            Self::Prefabs => "Prefabs",
            Self::Customers => "Clients",
            Self::System => "Système",
            Self::Extensions => "Extensions",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    /// Attribuable : au moins une route l'exige.
    Available,
    /// Nom réservé, feature à venir.
    Reserved,
}

macro_rules! permissions {
    ($($variant:ident => $key:literal, $category:ident, $label:literal, sensitive = $sensitive:literal, $availability:ident;)*) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum Permission {
            $($variant,)*
            /// Permission d'une extension (ADR-0051).
            Extension(ExtensionPermission),
        }

        impl Permission {
            pub const ALL: &'static [Permission] = &[$(Permission::$variant,)*];

            pub fn key(self) -> &'static str {
                match self {
                    $(Permission::$variant => $key,)*
                    Permission::Extension(permission) => permission.0.key,
                }
            }

            /// Permission de Kuro, ou d'une extension enregistrée.
            pub fn from_key(key: &str) -> Option<Self> {
                match key {
                    $($key => Some(Permission::$variant),)*
                    _ => registered(key).map(Permission::of),
                }
            }

            pub fn category(self) -> Category {
                match self {
                    $(Permission::$variant => Category::$category,)*
                    Permission::Extension(_) => Category::Extensions,
                }
            }

            pub fn label(self) -> &'static str {
                match self {
                    $(Permission::$variant => $label,)*
                    Permission::Extension(permission) => permission.0.label,
                }
            }

            pub fn is_sensitive(self) -> bool {
                match self {
                    $(Permission::$variant => $sensitive,)*
                    Permission::Extension(permission) => permission.0.sensitive,
                }
            }

            pub fn availability(self) -> Availability {
                match self {
                    $(Permission::$variant => Availability::$availability,)*
                    // Une extension ne déclare que des permissions qu'elle utilise.
                    Permission::Extension(_) => Availability::Available,
                }
            }
        }
    };
}

permissions! {
    // Accès et sécurité
    IamUsersRead => "iam.users.read", Iam, "Lister et consulter les comptes admin", sensitive = false, Available;
    IamUsersCreate => "iam.users.create", Iam, "Créer un compte admin", sensitive = false, Available;
    IamUsersUpdate => "iam.users.update", Iam, "Modifier nom et e-mail, activer ou désactiver un compte", sensitive = false, Available;
    IamUsersDelete => "iam.users.delete", Iam, "Supprimer un compte admin", sensitive = false, Available;
    IamUsersCredentials => "iam.users.credentials", Iam, "Réinitialiser mot de passe ou TOTP, révoquer les sessions", sensitive = true, Available;
    IamRolesRead => "iam.roles.read", Iam, "Consulter les rôles et le catalogue des permissions", sensitive = false, Available;
    IamRolesManage => "iam.roles.manage", Iam, "Créer, modifier, supprimer un rôle", sensitive = true, Available;
    IamRolesAssign => "iam.roles.assign", Iam, "Attribuer ou retirer des rôles", sensitive = true, Available;

    // Journal d'audit
    AuditRead => "audit.read", Audit, "Consulter le journal d'audit", sensitive = false, Available;

    // Infrastructure
    NodesRead => "nodes.read", Nodes, "Lister les nœuds, leur état et leur capacité", sensitive = false, Available;
    NodesCreate => "nodes.create", Nodes, "Ajouter un nœud et émettre son token d'enrôlement", sensitive = false, Available;
    NodesUpdate => "nodes.update", Nodes, "Modifier le libellé, la région et la localisation", sensitive = false, Available;
    NodesRegionsManage => "nodes.regions.manage", Nodes, "Créer, modifier, supprimer des régions", sensitive = false, Available;
    NodesEnrollment => "nodes.enrollment", Nodes, "Réémettre un token pour un nœud en attente", sensitive = true, Available;
    NodesRevoke => "nodes.revoke", Nodes, "Révoquer le certificat et couper le nœud", sensitive = true, Available;
    NodesDelete => "nodes.delete", Nodes, "Supprimer un nœud en attente ou hors ligne", sensitive = true, Available;
    NodesAllocationsRead => "nodes.allocations.read", Nodes, "Voir les ports alloués", sensitive = false, Available;
    NodesAllocationsManage => "nodes.allocations.manage", Nodes, "Créer ou supprimer des plages de ports", sensitive = false, Available;

    // Serveurs
    ServersRead => "servers.read", Servers, "Lister les serveurs, détail, état, statistiques", sensitive = false, Available;
    ServersCreate => "servers.create", Servers, "Créer et assigner un serveur", sensitive = false, Available;
    ServersUpdate => "servers.update", Servers, "Modifier les réglages et les ressources", sensitive = false, Available;
    ServersDelete => "servers.delete", Servers, "Supprimer un serveur et ses données", sensitive = true, Available;
    ServersPower => "servers.power", Servers, "Démarrer, arrêter, redémarrer, tuer", sensitive = false, Available;
    ServersReinstall => "servers.reinstall", Servers, "Réinstaller ou réinitialiser (efface les données)", sensitive = true, Available;
    ServersUpgrade => "servers.upgrade", Servers, "Mettre à jour vers une nouvelle version du prefab (autres images et scripts)", sensitive = true, Available;
    ServersConsoleRead => "servers.console.read", Servers, "Lire la console et les statistiques en direct", sensitive = false, Available;
    ServersConsoleWrite => "servers.console.write", Servers, "Envoyer des commandes à la console", sensitive = true, Available;
    ServersActionsRun => "servers.actions.run", Servers, "Lancer les actions déclarées par le prefab (commande dans le conteneur, dans le périmètre choisi par le prefab)", sensitive = true, Available;
    ServersFilesRead => "servers.files.read", Servers, "Lister et lire les fichiers (ils peuvent contenir des secrets : mot de passe RCON…)", sensitive = true, Available;
    ServersFilesWrite => "servers.files.write", Servers, "Écrire et supprimer des fichiers", sensitive = true, Available;
    ServersFilesSftp => "servers.files.sftp", Servers, "Ouvrir une session SFTP sur un serveur (avec la lecture, et l'écriture si accordée)", sensitive = true, Available;
    ServersBackupsRead => "servers.backups.read", Servers, "Lister les sauvegardes d'un serveur", sensitive = false, Available;
    ServersBackupsCreate => "servers.backups.create", Servers, "Créer, verrouiller ou déverrouiller une sauvegarde", sensitive = false, Available;
    ServersBackupsRestore => "servers.backups.restore", Servers, "Restaurer une sauvegarde (remplace les données)", sensitive = true, Available;
    ServersBackupsDelete => "servers.backups.delete", Servers, "Supprimer une sauvegarde", sensitive = true, Available;
    ServersBackupsDownload => "servers.backups.download", Servers, "Télécharger une sauvegarde (elle peut contenir des secrets)", sensitive = true, Available;
    ServersSchedulesRead => "servers.schedules.read", Servers, "Voir les tâches planifiées et leurs exécutions", sensitive = false, Available;
    ServersSchedulesManage => "servers.schedules.manage", Servers, "Créer, modifier, supprimer et lancer des tâches planifiées (seulement avec les actions qu'on détient)", sensitive = true, Available;
    ServersNotificationsRead => "servers.notifications.read", Servers, "Voir les abonnements aux notifications d'un serveur et leurs envois", sensitive = false, Available;
    ServersNotificationsManage => "servers.notifications.manage", Servers, "Créer, modifier, tester et supprimer les abonnements aux notifications d'un serveur", sensitive = false, Available;
    ServersContentRead => "servers.content.read", Servers, "Voir les mods et plugins installés et chercher dans les sources", sensitive = false, Available;
    ServersContentManage => "servers.content.manage", Servers, "Installer, mettre à jour, désactiver, supprimer des mods et plugins (code tiers exécuté)", sensitive = true, Available;
    ServersSuspend => "servers.suspend", Servers, "Suspendre ou réactiver un serveur", sensitive = false, Available;
    ServersTransfer => "servers.transfer", Servers, "Déplacer un serveur vers un autre nœud", sensitive = false, Reserved;

    // Catalogue
    PrefabsRead => "prefabs.read", Prefabs, "Consulter les prefabs (modèles de serveur)", sensitive = false, Available;
    PrefabsManage => "prefabs.manage", Prefabs, "Importer et supprimer des prefabs (ils font exécuter des images et des scripts)", sensitive = true, Available;

    // Clients
    CustomersRead => "customers.read", Customers, "Consulter les clients", sensitive = false, Reserved;
    CustomersUpdate => "customers.update", Customers, "Modifier un client", sensitive = false, Reserved;
    CustomersSuspend => "customers.suspend", Customers, "Suspendre un client", sensitive = false, Reserved;
    CustomersImpersonate => "customers.impersonate", Customers, "Agir en tant que client", sensitive = true, Reserved;

    // Système
    SystemSettingsRead => "system.settings.read", System, "Consulter les paramètres", sensitive = false, Available;
    SystemSettingsUpdate => "system.settings.update", System, "Modifier les paramètres", sensitive = true, Available;
    SystemDnsManage => "system.dns.manage", System, "Configurer le DNS (zone, jeton du fournisseur)", sensitive = true, Available;
    SystemContentSourcesManage => "system.content_sources.manage", System, "Configurer les sources de contenu (clé d'API CurseForge)", sensitive = true, Available;
    SystemSteamAccountsManage => "system.steam_accounts.manage", System, "Régler le compte Steam des installations", sensitive = true, Available;
    SystemNotificationsRead => "system.notifications.read", System, "Voir les canaux, les règles et le journal des notifications", sensitive = false, Available;
    SystemNotificationsManage => "system.notifications.manage", System, "Configurer les canaux (URL de webhook, secret) et les règles de notification", sensitive = true, Available;
}

impl Permission {
    /// Accordable à un agent IA (ADR-0023). Jamais : gestion des comptes et des rôles,
    /// paramètres de la plateforme, enrôlement, révocation et suppression des nœuds, suppressions
    /// et effacements de données, agir en tant que client.
    pub fn agent_grantable(self) -> bool {
        if let Self::Extension(permission) = self {
            return permission.0.agent_grantable;
        }
        !matches!(
            self,
            Self::IamUsersCreate
                | Self::IamUsersUpdate
                | Self::IamUsersDelete
                | Self::IamUsersCredentials
                | Self::IamRolesManage
                | Self::IamRolesAssign
                | Self::SystemSettingsUpdate
                | Self::SystemDnsManage
                | Self::SystemContentSourcesManage
                | Self::SystemSteamAccountsManage
                | Self::SystemNotificationsManage
                | Self::NodesEnrollment
                | Self::NodesRevoke
                | Self::NodesDelete
                | Self::ServersDelete
                | Self::ServersReinstall
                | Self::ServersBackupsRestore
                | Self::ServersBackupsDelete
                | Self::ServersFilesSftp
                | Self::CustomersImpersonate
        )
    }
}

/// Définition d'une permission d'extension, déclarée en `static` par l'extension :
/// `static CATALOG: PermissionDef = PermissionDef { key: "cloud.catalog.manage", … }`.
#[derive(Debug)]
pub struct PermissionDef {
    /// `<extension>.<ressource>.<action>` : préfixée par le nom de l'extension.
    pub key: &'static str,
    /// Libellé de repli (le panel traduit par la clé quand il connaît l'extension).
    pub label: &'static str,
    /// Action à fort impact, mise en évidence dans le panel.
    pub sensitive: bool,
    /// Accordable à un agent IA (ADR-0023). Faux sauf décision explicite de l'extension.
    pub agent_grantable: bool,
}

/// Référence à une permission d'extension enregistrée, comparée par sa clé.
#[derive(Clone, Copy)]
pub struct ExtensionPermission(&'static PermissionDef);

impl ExtensionPermission {
    pub fn key(self) -> &'static str {
        self.0.key
    }
}

impl std::fmt::Debug for ExtensionPermission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.key)
    }
}

impl PartialEq for ExtensionPermission {
    fn eq(&self, other: &Self) -> bool {
        self.0.key == other.0.key
    }
}

impl Eq for ExtensionPermission {}

impl std::hash::Hash for ExtensionPermission {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.key.hash(state);
    }
}

impl PartialOrd for ExtensionPermission {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ExtensionPermission {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.key.cmp(other.0.key)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PermissionError {
    #[error("permission {key:?} must start with {prefix:?} followed by lowercase segments")]
    InvalidKey { key: String, prefix: String },
    #[error("permission {0:?} is one of Kuro's")]
    Builtin(String),
    #[error("permission {0:?} has no label")]
    MissingLabel(String),
}

/// Permissions d'extensions enregistrées, par clé : écrites au démarrage, lues ensuite.
static REGISTRY: RwLock<BTreeMap<&'static str, &'static PermissionDef>> =
    RwLock::new(BTreeMap::new());

fn registered(key: &str) -> Option<&'static PermissionDef> {
    REGISTRY
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(key)
        .copied()
}

impl Permission {
    /// Permission d'une extension, pour ses gardes : `session.require(&iam, Permission::of(&DEF))`.
    pub fn of(def: &'static PermissionDef) -> Self {
        Self::Extension(ExtensionPermission(def))
    }

    /// Catalogue complet : celles de Kuro, puis celles des extensions enregistrées, par clé.
    pub fn all() -> Vec<Permission> {
        let mut all = Self::ALL.to_vec();
        all.extend(
            REGISTRY
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .values()
                .map(|def| Self::of(def)),
        );
        all
    }

    /// Enregistre les permissions de l'extension `extension` : chaque clé est préfixée par son
    /// nom (`<extension>.x.y`, segments `[a-z][a-z0-9_]*`) et distincte de celles de Kuro.
    /// Ré-enregistrer une clé la remplace (redémarrage, tests).
    pub fn register(
        extension: &str,
        defs: &'static [PermissionDef],
    ) -> Result<(), PermissionError> {
        for def in defs {
            check_extension_key(extension, def)?;
        }
        let mut registry = REGISTRY
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for def in defs {
            registry.insert(def.key, def);
        }
        Ok(())
    }
}

fn check_extension_key(extension: &str, def: &PermissionDef) -> Result<(), PermissionError> {
    let prefix = format!("{extension}.");
    let invalid = || PermissionError::InvalidKey {
        key: def.key.to_owned(),
        prefix: prefix.clone(),
    };
    let rest = def.key.strip_prefix(&prefix).ok_or_else(invalid)?;
    let segment_ok = |segment: &str| {
        segment.starts_with(|c: char| c.is_ascii_lowercase())
            && segment
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    };
    if rest.is_empty() || !rest.split('.').all(segment_ok) {
        return Err(invalid());
    }
    if Permission::ALL.iter().any(|p| p.key() == def.key) {
        return Err(PermissionError::Builtin(def.key.to_owned()));
    }
    if def.label.trim().is_empty() {
        return Err(PermissionError::MissingLabel(def.key.to_owned()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    static NOTES_READ: PermissionDef = PermissionDef {
        key: "unittest.notes.read",
        label: "Lire les notes",
        sensitive: false,
        agent_grantable: false,
    };
    static NOTES_PURGE: PermissionDef = PermissionDef {
        key: "unittest.notes.purge",
        label: "Purger les notes",
        sensitive: true,
        agent_grantable: true,
    };

    fn defs(key: &'static str) -> &'static [PermissionDef] {
        Box::leak(Box::new([PermissionDef {
            key,
            label: "x",
            sensitive: false,
            agent_grantable: false,
        }]))
    }

    #[test]
    fn registered_permissions_join_the_catalog() {
        Permission::register("unittest", std::slice::from_ref(&NOTES_READ)).unwrap();
        Permission::register("unittest", std::slice::from_ref(&NOTES_PURGE)).unwrap();

        let read = Permission::from_key("unittest.notes.read").unwrap();
        assert_eq!(read, Permission::of(&NOTES_READ));
        assert_eq!(read.key(), "unittest.notes.read");
        assert_eq!(read.category(), Category::Extensions);
        assert_eq!(read.availability(), Availability::Available);
        assert!(!read.agent_grantable());
        assert!(Permission::of(&NOTES_PURGE).agent_grantable());
        assert!(Permission::of(&NOTES_PURGE).is_sensitive());
        assert!(Permission::all().contains(&read));
    }

    #[test]
    fn an_extension_key_carries_its_prefix_and_lowercase_segments() {
        for key in [
            "other.notes.read",
            "unittest",
            "unittest.",
            "unittest.Notes",
            "unittest..x",
            "unittest.1x",
        ] {
            assert!(
                matches!(
                    Permission::register("unittest", defs(key)),
                    Err(PermissionError::InvalidKey { .. })
                ),
                "{key}"
            );
        }
    }

    #[test]
    fn an_extension_cannot_redefine_a_kuro_permission() {
        assert_eq!(
            Permission::register("servers", defs("servers.read")),
            Err(PermissionError::Builtin("servers.read".to_owned()))
        );
    }

    #[test]
    fn an_unknown_key_is_still_unknown() {
        assert_eq!(Permission::from_key("unittest.nothing.here"), None);
    }
}
