//! Catalogue des permissions de l'espace admin : **source de vérité** (ADR-0007).
//!
//! Format `domaine.ressource.action`. Une permission `Reserved` existe pour figer son nom et son
//! périmètre, mais n'est pas attribuable tant qu'aucune route ne l'utilise.
//! Une permission sensible (`sensitive = true`) est mise en évidence dans le panel.

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
        }

        impl Permission {
            pub const ALL: &'static [Permission] = &[$(Permission::$variant,)*];

            pub fn key(self) -> &'static str {
                match self {
                    $(Permission::$variant => $key,)*
                }
            }

            pub fn from_key(key: &str) -> Option<Self> {
                match key {
                    $($key => Some(Permission::$variant),)*
                    _ => None,
                }
            }

            pub fn category(self) -> Category {
                match self {
                    $(Permission::$variant => Category::$category,)*
                }
            }

            pub fn label(self) -> &'static str {
                match self {
                    $(Permission::$variant => $label,)*
                }
            }

            pub fn is_sensitive(self) -> bool {
                match self {
                    $(Permission::$variant => $sensitive,)*
                }
            }

            pub fn availability(self) -> Availability {
                match self {
                    $(Permission::$variant => Availability::$availability,)*
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
    NodesDelete => "nodes.delete", Nodes, "Supprimer un nœud (et couper sa session)", sensitive = true, Available;
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
    ServersBackupsRead => "servers.backups.read", Servers, "Lister les sauvegardes d'un serveur", sensitive = false, Available;
    ServersBackupsCreate => "servers.backups.create", Servers, "Créer, verrouiller ou déverrouiller une sauvegarde", sensitive = false, Available;
    ServersBackupsRestore => "servers.backups.restore", Servers, "Restaurer une sauvegarde (remplace les données)", sensitive = true, Available;
    ServersBackupsDelete => "servers.backups.delete", Servers, "Supprimer une sauvegarde", sensitive = true, Available;
    ServersBackupsDownload => "servers.backups.download", Servers, "Télécharger une sauvegarde (elle peut contenir des secrets)", sensitive = true, Available;
    ServersSchedulesRead => "servers.schedules.read", Servers, "Voir les tâches planifiées et leurs exécutions", sensitive = false, Available;
    ServersSchedulesManage => "servers.schedules.manage", Servers, "Créer, modifier, supprimer et lancer des tâches planifiées (seulement avec les actions qu'on détient)", sensitive = true, Available;
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
}

impl Permission {
    /// Accordable à un agent IA (ADR-0023). Jamais : gestion des comptes et des rôles,
    /// paramètres de la plateforme, enrôlement, révocation et suppression des nœuds, suppressions
    /// et effacements de données, agir en tant que client.
    pub fn agent_grantable(self) -> bool {
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
                | Self::NodesEnrollment
                | Self::NodesRevoke
                | Self::NodesDelete
                | Self::ServersDelete
                | Self::ServersReinstall
                | Self::ServersBackupsRestore
                | Self::ServersBackupsDelete
                | Self::CustomersImpersonate
        )
    }
}
