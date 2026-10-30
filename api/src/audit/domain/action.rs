//! Catalogue des actions auditées : codes stables, à ne jamais renommer (ils sont stockés).

macro_rules! audit_actions {
    ($($variant:ident => $code:literal,)*) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum AuditAction {
            $($variant,)*
        }

        impl AuditAction {
            pub const ALL: &'static [AuditAction] = &[$(AuditAction::$variant,)*];

            pub fn code(self) -> &'static str {
                match self {
                    $(AuditAction::$variant => $code,)*
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
    TotpSetupStarted => "auth.totp.setup_started",
    TotpEnabled => "auth.totp.enabled",
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
    OwnPasswordChanged => "iam.self.password_changed",

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
    ServerFileWritten => "server.file_written",
    ServerDirectoryCreated => "server.directory_created",
    ServerFileRenamed => "server.file_renamed",
    ServerFileDeleted => "server.file_deleted",
    ServerFileUploaded => "server.file_uploaded",
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

    // Paramètres du projet
    SettingsUpdated => "system.settings_updated",

    // Journal lui-même
    AuditRead => "audit.read",
    AuditPurged => "audit.purged",
    AuditVerified => "audit.verified",
    AuditExported => "audit.exported",
}
