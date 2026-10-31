//! Catalogue des codes d'erreur de l'API : **source de vérité** du contrat d'erreur.
//!
//! Exporté dans l'OpenAPI comme énumération fermée : le panel en dérive un type TypeScript et ses
//! fichiers de traduction doivent couvrir chaque code (vérifié à la compilation du front).
//! Le titre anglais n'est qu'un repli, la traduction se fait côté client. Codes stables : ne
//! jamais renommer un code publié.

use serde::Serialize;
use serde_json::{Map, Value};
use std::borrow::Cow;
use utoipa::ToSchema;

macro_rules! error_codes {
    ($($variant:ident => $code:literal, $title:literal;)*) => {
        // `rename_all` (et non un `rename` par variante) : c'est ce que reprend le schéma OpenAPI.
        // Le test `every_error_code_is_published_in_the_openapi_document` vérifie l'accord avec
        // les codes littéraux ci-dessous.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, ToSchema)]
        #[serde(rename_all = "snake_case")]
        pub enum ErrorCode {
            $($variant,)*
        }

        impl ErrorCode {
            pub const ALL: &'static [ErrorCode] = &[$(ErrorCode::$variant,)*];

            pub fn as_str(self) -> &'static str {
                match self {
                    $(ErrorCode::$variant => $code,)*
                }
            }

            /// Titre anglais de repli (RFC 9457 `title`).
            pub fn title(self) -> &'static str {
                match self {
                    $(ErrorCode::$variant => $title,)*
                }
            }
        }
    };
}

error_codes! {
    // Génériques
    ValidationFailed => "validation_failed", "Validation failed";
    InvalidBody => "invalid_body", "Malformed request body";
    InvalidParameter => "invalid_parameter", "Invalid path or query parameter";
    EmptyPatch => "empty_patch", "Nothing to update";
    NotFound => "not_found", "Not found";
    Unauthorized => "unauthorized", "Authentication required";
    Forbidden => "forbidden", "Permission required";
    InternalError => "internal_error", "Internal error";
    RateLimited => "rate_limited", "Too many requests, try again later";
    RequestTimeout => "request_timeout", "The request took too long";

    // Authentification
    InvalidCredentials => "invalid_credentials", "Invalid credentials";
    InvalidChallenge => "invalid_challenge", "Invalid or expired challenge";
    InvalidCode => "invalid_code", "Invalid code";
    TotpAlreadyEnabled => "totp_already_enabled", "TOTP is already enabled";
    TotpSetupNotStarted => "totp_setup_not_started", "TOTP setup has not been started";

    // Comptes et rôles
    InvalidEmail => "invalid_email", "Invalid email address";
    InvalidDisplayName => "invalid_display_name", "Invalid display name";
    InvalidProjectName => "invalid_project_name", "Project name must be 1 to 80 characters";
    // Première mise en route (ADR-0045)
    SetupDone => "setup_done", "Setup already done: an admin account exists";
    SetupTokenUnavailable => "setup_token_unavailable", "Setup token unavailable on the host: use `kuro-api setup`";
    WeakPassword => "weak_password", "Password does not meet the policy";
    PasswordUnchanged => "password_unchanged", "The new password must differ from the current one";
    EmailTaken => "email_taken", "Email already in use";
    CannotDisableSelf => "cannot_disable_self", "You cannot disable your own account";
    CannotDeleteSelf => "cannot_delete_self", "You cannot delete your own account";
    CannotChangeOwnRoles => "cannot_change_own_roles", "You cannot change your own roles";
    UseOwnPasswordChange => "use_own_password_change", "Change your own password from your account";
    CannotResetOwnTotp => "cannot_reset_own_totp", "You cannot reset your own TOTP";
    InsufficientPermissionsForTarget => "insufficient_permissions_for_target", "This account has permissions you do not hold";
    InsufficientPermissionsForRole => "insufficient_permissions_for_role", "This role has permissions you do not hold";
    CannotGrantPermissionsYouLack => "cannot_grant_permissions_you_lack", "You cannot grant permissions you do not hold";
    LastSuperAdmin => "last_super_admin", "The last active super-admin must remain";
    UnknownRole => "unknown_role", "Unknown role";
    InvalidRoleName => "invalid_role_name", "Invalid role name";
    InvalidDescription => "invalid_description", "Invalid description";
    InvalidColor => "invalid_color", "Invalid color";
    RoleNameTaken => "role_name_taken", "Role name already in use";
    SystemRole => "system_role", "System roles cannot be changed";
    UnknownPermission => "unknown_permission", "Unknown permission";
    PermissionNotAssignable => "permission_not_assignable", "Permission cannot be assigned";

    // Nœuds et régions
    InvalidNodeName => "invalid_node_name", "Invalid node name";
    NodeNameTaken => "node_name_taken", "Node name already in use";
    UnknownRegion => "unknown_region", "Unknown region";
    InvalidRegionSlug => "invalid_region_slug", "Invalid region slug";
    InvalidRegionName => "invalid_region_name", "Invalid region name";
    RegionSlugTaken => "region_slug_taken", "Region slug already in use";
    RegionNotEmpty => "region_not_empty", "The region still contains nodes";
    InvalidCoordinates => "invalid_coordinates", "Invalid coordinates";
    InvalidLocation => "invalid_location", "Invalid location";
    InvalidReason => "invalid_reason", "Invalid reason";
    NodeNotPending => "node_not_pending", "The node is already enrolled";
    NodeAlreadyRevoked => "node_already_revoked", "The node is already revoked";
    NodeNotEnrolled => "node_not_enrolled", "The node is not enrolled yet";

    // Enrôlement et lien daemon
    InvalidToken => "invalid_token", "Invalid, expired or already used token";
    InvalidCsr => "invalid_csr", "Invalid certificate signing request";
    AlreadyEnrolled => "already_enrolled", "This host is already enrolled";
    ClientCertificateRequired => "client_certificate_required", "Client certificate required";
    UnknownNode => "unknown_node", "Unknown node certificate";
    NodeRevoked => "node_revoked", "The node has been revoked";
    // Allocations de ports
    InvalidPortRange => "invalid_port_range", "Invalid port range";
    InvalidIp => "invalid_ip", "Invalid IP address";
    NoFreeAllocation => "no_free_allocation", "No free port on this node";
    UnknownAllocation => "unknown_allocation", "Unknown allocation";
    AllocationTaken => "allocation_taken", "This port is already assigned";
    AllocationInUse => "allocation_in_use", "This port is assigned to a server";
    NodeHasServers => "node_has_servers", "The node still hosts servers or allocations";
    // Prefabs et serveurs
    UnknownPrefab => "unknown_prefab", "Unknown prefab or version";
    InvalidPrefab => "invalid_prefab", "Invalid prefab or archive";
    PrefabExists => "prefab_exists", "This prefab version is already imported";
    StoreUnavailable => "store_unavailable", "The prefab store cannot be reached";
    StoreUntrusted => "store_untrusted", "The prefab store index is missing, invalid or not signed by the expected key";
    StoreCorrupted => "store_corrupted", "The downloaded archive does not match the store index";
    PrefabInUse => "prefab_in_use", "Servers still use this prefab version";
    InvalidSetting => "invalid_setting", "Invalid setting value";
    SettingNotEditable => "setting_not_editable", "This setting is fixed after deployment";
    InvalidServerName => "invalid_server_name", "Invalid server name";
    InvalidResources => "invalid_resources", "Invalid resources";
    NodeOffline => "node_offline", "The node is offline";
    DaemonTooOld => "daemon_too_old", "The daemon of this node is too old for this prefab: update kuro-daemon on the node";
    ConsoleHeldElsewhere => "console_held_elsewhere", "This node is linked to another API instance: its console is not available here";
    NodeNotForTrials => "node_not_for_trials", "This node does not accept Studio trials";
    PrefabDraftNeedsTestNode => "prefab_draft_needs_test_node", "This prefab version is a draft: deploy it on a test node";
    // DNS (ADR-0026)
    DnsNotConfigured => "dns_not_configured", "DNS is not configured";
    InvalidDomain => "invalid_domain", "Invalid domain name";
    InvalidDnsLabel => "invalid_dns_label", "Invalid name: lowercase letters, digits and dashes";
    DnsLabelTaken => "dns_label_taken", "This name is already used by another server";
    DnsLabelReserved => "dns_label_reserved", "This name is reserved (panel, API, nodes…)";
    // Sites web (ADR-0038)
    InvalidSitesDomain => "invalid_sites_domain", "Invalid web sites domain: in the zone, apart from the server and node domains, never a reserved name";
    SitesDomainNotConfigured => "sites_domain_not_configured", "No web sites domain is configured in the DNS settings";
    InvalidIpAddress => "invalid_ip_address", "Invalid IP address";
    UnknownDnsProvider => "unknown_dns_provider", "Unknown DNS provider";
    DnsTokenRequired => "dns_token_required", "An API token is required";
    DnsProviderUnauthorized => "dns_provider_unauthorized", "The DNS provider refused the token";
    DnsZoneNotFound => "dns_zone_not_found", "No zone of the DNS provider holds this domain";
    DnsProviderUnavailable => "dns_provider_unavailable", "The DNS provider did not answer";
    UpgradeBlocked => "upgrade_blocked", "This upgrade is blocked: see its preview";
    UpgradeNotNewer => "upgrade_not_newer", "Only a newer version of the same prefab can be installed";
    // Agents IA (ADR-0023)
    OauthNoScope => "oauth_no_scope", "None of the requested rights can be granted";
    ServerBusy => "server_busy", "An operation is already running on this server";
    ServerNotRunning => "server_not_running", "The server is not running";
    InvalidCommand => "invalid_command", "A console command is a single line without control characters";
    // Actions d'un prefab (ADR-0039)
    UnknownAction => "unknown_action", "This prefab declares no such action";
    InvalidActionParam => "invalid_action_param", "Invalid action parameter";
    ActionNotSchedulable => "action_not_schedulable", "This action cannot be scheduled: not declared schedulable, or it needs parameters";
    InvalidPath => "invalid_path", "Invalid path: it must stay under a volume of the server";
    FileNotFound => "file_not_found", "No such file or directory";
    FileExists => "file_exists", "A file or directory with this name already exists";
    FileConflict => "file_conflict", "The file changed since it was opened";
    FileNotText => "file_not_text", "Not a text file";
    FileTooLarge => "file_too_large", "File too large to be edited";
    BackupNotFound => "backup_not_found", "No such backup";
    InvalidBackupName => "invalid_backup_name", "Invalid backup name";
    BackupsDisabled => "backups_disabled", "Backups are disabled for this server";
    BackupLimitReached => "backup_limit_reached", "Backup limit reached and every backup is locked";
    BackupInProgress => "backup_in_progress", "A backup of this server is already running";
    BackupNotReady => "backup_not_ready", "This backup is not complete";
    BackupLocked => "backup_locked", "This backup is locked";
    ScheduleNotFound => "schedule_not_found", "No such scheduled task";
    InvalidMetricsRange => "invalid_metrics_range", "Range must be 24h, 7d or 90d";
    NotificationChannelNotFound => "notification_channel_not_found", "No such notification channel";
    NotificationRuleNotFound => "notification_rule_not_found", "No such notification rule";
    InvalidNotificationName => "invalid_notification_name", "Name must be 1 to 80 visible characters";
    InvalidNotificationChannelKind => "invalid_notification_channel_kind", "Channel kind must be webhook, discord or slack";
    InvalidNotificationLanguage => "invalid_notification_language", "Language must be fr or en";
    InvalidWebhookUrl => "invalid_webhook_url", "Webhook URL must be https:// without credentials";
    InvalidNotificationScope => "invalid_notification_scope", "Scope must be all, or node/server with an existing target";
    InvalidNotificationEvents => "invalid_notification_events", "At least one known event is required";
    InvalidNotificationChannels => "invalid_notification_channels", "At least one existing channel is required";
    InvalidScheduleName => "invalid_schedule_name", "Invalid scheduled task name";
    InvalidCron => "invalid_cron", "Invalid cron expression (5 fields) or one that never runs";
    InvalidTimezone => "invalid_timezone", "Unknown time zone";
    InvalidScheduleStep => "invalid_schedule_step", "Invalid scheduled task step";
    ScheduleLimitReached => "schedule_limit_reached", "This server has reached its number of scheduled tasks";
    ScheduleRunning => "schedule_running", "This scheduled task is already running";
    ResourceAboveCeiling => "resource_above_ceiling", "Above the ceiling set in the project settings";
    ResourceBelowPrefabMinimum => "resource_below_prefab_minimum", "Below the minimum this prefab needs to run";
    DiskQuotaExceeded => "disk_quota_exceeded", "The disk quota of this server is reached";
    ServerSuspended => "server_suspended", "This server is suspended";
    // Contenu (mods, plugins)
    ContentSlotNotFound => "content_slot_not_found", "No such content slot for this server";
    ContentNotSupported => "content_not_supported", "This server cannot load this content (loader)";
    GameVersionNotPinned => "game_version_not_pinned", "Pin the game version before adding content";
    ContentNotFound => "content_not_found", "No such content";
    ContentAlreadyInstalled => "content_already_installed", "This project is already installed";
    ContentIncompatible => "content_incompatible", "No version compatible with this server";
    ContentRequired => "content_required", "Other content still requires this one";
    ContentUpToDate => "content_up_to_date", "Already the latest compatible version";
    ProviderUnavailable => "provider_unavailable", "The content provider did not answer";
    ModpackInstalled => "modpack_installed", "A modpack is already installed: remove it first";
    ContentInModpack => "content_in_modpack", "This file belongs to a modpack";
    InvalidModpack => "invalid_modpack", "Unreadable or unsupported modpack";
    ContentDistributionForbidden => "content_distribution_forbidden", "The author does not allow third-party downloads of this content";
    ContentSourceUnsupported => "content_source_unsupported", "Kuro does not support this content source yet";
    UploadNotAccepted => "upload_not_accepted", "This slot does not accept uploaded files";
    ContentValueRequired => "content_value_required", "This value is required";
    InvalidContentValue => "invalid_content_value", "Invalid value for this content";
    // Sources de contenu à clé d'API (ADR-0029)
    UnknownContentSource => "unknown_content_source", "No such content source";
    ContentSourceNotConfigured => "content_source_not_configured", "This content source has no API key";
    ContentSourceKeyRequired => "content_source_key_required", "An API key is required";
    ContentSourceUnauthorized => "content_source_unauthorized", "The content source refused this API key";
    // Compte Steam d'installation (ADR-0033)
    SteamAccountRequired => "steam_account_required", "This prefab needs the Steam account of the installation";
    SteamAccountNotConfigured => "steam_account_not_configured", "No Steam account is set";
    SteamAccountUsernameInvalid => "steam_account_username_invalid", "A Steam account name has 3 to 64 letters, digits or underscores";
    SteamAccountPasswordInvalid => "steam_account_password_invalid", "Up to 64 characters, without quotes, backslashes or control characters";
    // Images d'un jeu Steam dans le Studio (ADR-0039)
    SteamGameMissing => "steam_game_missing", "The saved manifest declares no Steam game (steam.app)";
    SteamGameNotFound => "steam_game_not_found", "No game with this id on the Steam store";
    // Paramètres du projet
    ValueOutOfRange => "value_out_of_range", "Value out of range";
    DaemonTimeout => "daemon_timeout", "The node did not answer in time";
    DaemonError => "daemon_error", "The node refused the command";
}

/// Erreur rattachée à un champ de la requête, affichée sous ce champ dans les formulaires.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldError {
    /// Nom du champ ; `settings.<clé>` pour un réglage de prefab.
    pub field: Cow<'static, str>,
    pub code: ErrorCode,
    pub params: Map<String, Value>,
}

impl FieldError {
    pub fn new(field: impl Into<Cow<'static, str>>, code: ErrorCode) -> Self {
        Self {
            field: field.into(),
            code,
            params: Map::new(),
        }
    }

    pub fn param(mut self, name: &str, value: impl Into<Value>) -> Self {
        self.params.insert(name.to_owned(), value.into());
        self
    }
}
