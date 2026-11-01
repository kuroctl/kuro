//! Catalogue des actions auditées : codes stables, à ne jamais renommer (ils sont stockés).
//! Une extension compilée avec Kuro (ADR-0051) ajoute les siennes, préfixées par son nom
//! (`cloud.catalog.product_created`) : [`AuditAction::Extension`].

use std::collections::BTreeMap;
use std::sync::RwLock;

macro_rules! audit_actions {
    ($($variant:ident => $code:literal,)*) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum AuditAction {
            $($variant,)*
            /// Action d'une extension (ADR-0051).
            Extension(ExtensionAuditAction),
        }

        impl AuditAction {
            pub const ALL: &'static [AuditAction] = &[$(AuditAction::$variant,)*];

            pub fn code(self) -> &'static str {
                match self {
                    $(AuditAction::$variant => $code,)*
                    AuditAction::Extension(action) => action.0.code,
                }
            }
        }
    };
}

audit_actions! {
    // Authentification
    LoginSucceeded => "auth.login.succeeded",
    LoginFailed => "auth.login.failed",
    LoginPasswordChangeRequired => "auth.login.password_change_required",
    AccountLocked => "auth.login.locked",
    TotpFailed => "auth.totp.failed",
    PasswordResetRequested => "auth.password_reset.requested",
    PasswordResetCompleted => "auth.password_reset.completed",
    ElevationGranted => "auth.elevation.granted",
    TotpSetupStarted => "auth.totp.setup_started",
    TotpEnabled => "auth.totp.enabled",
    TotpReplaced => "auth.totp.replaced",
    TotpDisabled => "auth.totp.disabled",
    RecoveryCodeUsed => "auth.totp.recovery_code_used",
    RecoveryCodesRegenerated => "auth.totp.recovery_codes_regenerated",
    SftpAuthFailed => "auth.sftp.failed",
    SessionRefreshed => "auth.session.refreshed",
    SessionReuseDetected => "auth.session.reuse_detected",
    Logout => "auth.logout",

    // Agents IA : serveur d'autorisation OAuth du MCP (ADR-0023)
    // DNS des serveurs et des nœuds (ADR-0026)
    DnsZoneUpdated => "dns.zone_updated",
    DnsZoneRemoved => "dns.zone_removed",
    DnsRecordCreated => "dns.record_created",
    DnsRecordUpdated => "dns.record_updated",
    DnsRecordDeleted => "dns.record_deleted",
    NodeAddressUpdated => "node.address_updated",
    // Sources de contenu à clé d'API (ADR-0029)
    ContentSourceUpdated => "content_source.updated",
    ContentSourceRemoved => "content_source.removed",
    // Compte Steam d'installation (ADR-0033)
    SteamAccountUpdated => "steam_account.updated",
    SteamAccountRemoved => "steam_account.removed",

    OAuthClientRegistered => "oauth.client_registered",
    OAuthGrantApproved => "oauth.grant_approved",
    OAuthGrantDenied => "oauth.grant_denied",
    OAuthGrantRevoked => "oauth.grant_revoked",
    OAuthTokenIssued => "oauth.token_issued",
    OAuthTokenRefreshed => "oauth.token_refreshed",
    OAuthRefreshReused => "oauth.refresh_reused",

    // Comptes admin
    UserCreated => "iam.user.created",
    UserUpdated => "iam.user.updated",
    UserDisabled => "iam.user.disabled",
    UserEnabled => "iam.user.enabled",
    UserDeleted => "iam.user.deleted",
    UserRolesChanged => "iam.user.roles_changed",
    UserPasswordReset => "iam.user.password_reset",
    UserTotpReset => "iam.user.totp_reset",
    UserSessionsRevoked => "iam.user.sessions_revoked",
    UserInvited => "iam.user.invited",
    InvitationAccepted => "iam.invitation.accepted",
    OwnPasswordChanged => "iam.self.password_changed",
    OwnSshKeyAdded => "iam.self.ssh_key_added",
    OwnSshKeyRemoved => "iam.self.ssh_key_removed",

    // Rôles
    RoleCreated => "iam.role.created",
    RoleUpdated => "iam.role.updated",
    RoleDeleted => "iam.role.deleted",

    // Contrôle d'accès
    AccessDenied => "access.denied",

    // Infrastructure
    PkiInitialized => "pki.initialized",
    NodeAdded => "node.added",
    NodeEnrolled => "node.enrolled",
    NodeEnrollmentReissued => "node.enrollment_reissued",
    NodeUpdated => "node.updated",
    NodeRevoked => "node.revoked",
    NodeDeleted => "node.deleted",
    RegionCreated => "region.created",
    RegionUpdated => "region.updated",
    RegionDeleted => "region.deleted",
    AllocationsCreated => "allocation.created",
    AllocationDeleted => "allocation.deleted",

    // Prefabs et serveurs
    PrefabImported => "prefab.imported",
    PrefabAssetsImported => "prefab.assets_imported",
    PrefabDeleted => "prefab.deleted",
    PrefabAssetsOptimized => "prefab.assets_optimized",
    PrefabDraftCreated => "prefab_draft.created",
    PrefabDraftUpdated => "prefab_draft.updated",
    PrefabDraftDeleted => "prefab_draft.deleted",
    PrefabDraftPublished => "prefab_draft.published",
    PrefabDraftTrialStarted => "prefab_draft.trial_started",
    PrefabDraftTrialEnded => "prefab_draft.trial_ended",
    PrefabsReloaded => "prefabs.reloaded",
    ServerCreated => "server.created",
    ServerSettingsUpdated => "server.settings_updated",
    ServerPower => "server.power",
    ServerDeleted => "server.deleted",
    ServerReinstalled => "server.reinstalled",
    ServerUpgraded => "server.upgraded",
    ServerDnsLabelChanged => "server.dns_label_changed",
    ServerConsoleOpened => "server.console_opened",
    ServerCommand => "server.command",
    ServerActionRan => "server.action_ran",
    ServerFileWritten => "server.file_written",
    ServerDirectoryCreated => "server.directory_created",
    ServerFileRenamed => "server.file_renamed",
    ServerFileDeleted => "server.file_deleted",
    ServerFileUploaded => "server.file_uploaded",
    ServerFileModeChanged => "server.file_mode_changed",
    ServerFileRestored => "server.file_restored",
    SftpSessionOpened => "sftp.session.opened",
    SftpSessionClosed => "sftp.session.closed",
    ServerBackupCreated => "server.backup_created",
    ServerBackupLocked => "server.backup_locked",
    ServerBackupRestored => "server.backup_restored",
    ServerBackupDeleted => "server.backup_deleted",
    ServerBackupDownloaded => "server.backup_downloaded",
    ServerScheduleCreated => "server.schedule_created",
    ServerScheduleUpdated => "server.schedule_updated",
    ServerScheduleDeleted => "server.schedule_deleted",
    ServerScheduleStarted => "server.schedule_started",
    ServerScheduleRun => "server.schedule_run",
    ServerResourcesUpdated => "server.resources_updated",
    ServerContentInstalled => "server.content_installed",
    ServerContentUpdated => "server.content_updated",
    ServerContentToggled => "server.content_toggled",
    ServerContentRemoved => "server.content_removed",
    ServerContentValuesUpdated => "server.content_values_updated",
    ServerModpackInstalled => "server.modpack_installed",
    ServerSuspended => "server.suspended",
    ServerUnsuspended => "server.unsuspended",
    // Propriétaire d'un serveur (ADR-0051)
    ServerOwnerChanged => "server.owner_changed",

    // Paramètres du projet
    SettingsUpdated => "system.settings_updated",
    // Première mise en route (ADR-0045)
    SetupCompleted => "system.setup_completed",
    // E-mail (ADR-0046)
    MailUpdated => "system.mail_updated",
    MailRemoved => "system.mail_removed",
    MailTested => "system.mail_tested",

    // Notifications (ADR-0044)
    NotificationChannelCreated => "notification.channel_created",
    NotificationChannelUpdated => "notification.channel_updated",
    NotificationChannelDeleted => "notification.channel_deleted",
    NotificationChannelTested => "notification.channel_tested",
    NotificationRuleCreated => "notification.rule_created",
    NotificationRuleUpdated => "notification.rule_updated",
    NotificationRuleDeleted => "notification.rule_deleted",

    // Journal lui-même
    AuditRead => "audit.read",
    AuditPurged => "audit.purged",
    AuditVerified => "audit.verified",
    AuditExported => "audit.exported",
}

/// Action d'audit d'une extension, déclarée en `static` :
/// `static CREATED: AuditActionDef = AuditActionDef { code: "cloud.catalog.product_created" }`.
#[derive(Debug)]
pub struct AuditActionDef {
    /// `<extension>.<ressource>.<action>`, stable : il est stocké dans le journal.
    pub code: &'static str,
}

/// Référence à une action d'extension enregistrée, comparée par son code.
#[derive(Clone, Copy)]
pub struct ExtensionAuditAction(&'static AuditActionDef);

impl std::fmt::Debug for ExtensionAuditAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.code)
    }
}

impl PartialEq for ExtensionAuditAction {
    fn eq(&self, other: &Self) -> bool {
        self.0.code == other.0.code
    }
}

impl Eq for ExtensionAuditAction {}

impl std::hash::Hash for ExtensionAuditAction {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.code.hash(state);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuditActionError {
    #[error(
        "audit action {code:?} must start with \"{extension}.\" followed by lowercase segments"
    )]
    InvalidCode { code: String, extension: String },
    #[error("audit action {0:?} is one of Kuro's")]
    Builtin(String),
}

/// Actions d'extensions enregistrées, par code : écrites au démarrage, lues ensuite.
static REGISTRY: RwLock<BTreeMap<&'static str, &'static AuditActionDef>> =
    RwLock::new(BTreeMap::new());

impl AuditAction {
    /// Action d'une extension, pour son audit : `AuditEvent::success(AuditAction::of(&DEF))`.
    pub fn of(def: &'static AuditActionDef) -> Self {
        Self::Extension(ExtensionAuditAction(def))
    }

    /// Actions de Kuro, puis celles des extensions enregistrées, par code.
    pub fn all() -> Vec<AuditAction> {
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

    /// Enregistre les actions de l'extension `extension` (codes préfixés par son nom, distincts
    /// de ceux de Kuro). Ré-enregistrer un code le remplace (redémarrage, tests).
    pub fn register(
        extension: &str,
        defs: &'static [AuditActionDef],
    ) -> Result<(), AuditActionError> {
        for def in defs {
            if !crate::extension_key::is_extension_key(extension, def.code) {
                return Err(AuditActionError::InvalidCode {
                    code: def.code.to_owned(),
                    extension: extension.to_owned(),
                });
            }
            if Self::ALL.iter().any(|action| action.code() == def.code) {
                return Err(AuditActionError::Builtin(def.code.to_owned()));
            }
        }
        let mut registry = REGISTRY
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for def in defs {
            registry.insert(def.code, def);
        }
        Ok(())
    }
}
