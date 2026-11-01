//! Accès à la ligne `platform_settings`.

use sqlx::PgConnection;
use time::OffsetDateTime;

/// Paramètres stockés. Un plafond `None` : aucun. Disque `0` : illimité.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformSettings {
    pub default_memory_mb: i32,
    pub max_memory_mb: Option<i32>,
    pub default_cpu_percent: i32,
    pub max_cpu_percent: Option<i32>,
    pub default_disk_mb: i32,
    pub max_disk_mb: Option<i32>,
    pub default_backup_limit: i32,
    pub max_backup_limit: Option<i32>,
    pub audit_retention_days: i32,
    /// Serveur MCP ouvert aux agents IA (ADR-0023).
    pub mcp_enabled: bool,
    /// Accès SFTP aux fichiers des serveurs (ADR-0047).
    pub sftp_enabled: bool,
    /// Historique des fichiers (ADR-0049) : jours (0 : coupé) et Mio par serveur.
    pub file_history_days: i32,
    pub file_history_max_mb: i32,
    /// Seuils d'alerte des stats (ADR-0043) : CPU et mémoire en % de l'allocation, soutenus
    /// pendant `alert_minutes` minutes.
    pub alert_cpu_percent: i32,
    pub alert_memory_percent: i32,
    pub alert_minutes: i32,
    /// Nom du projet (ADR-0045) ; `None` : « Kuro ».
    pub project_name: Option<String>,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug)]
pub struct SettingsRepository;

impl SettingsRepository {
    pub async fn get(conn: &mut PgConnection) -> Result<PlatformSettings, sqlx::Error> {
        sqlx::query_as!(
            PlatformSettings,
            "SELECT default_memory_mb, max_memory_mb, default_cpu_percent, max_cpu_percent,
                    default_disk_mb, max_disk_mb, default_backup_limit, max_backup_limit,
                    audit_retention_days, mcp_enabled, sftp_enabled, file_history_days,
                    file_history_max_mb, alert_cpu_percent, alert_memory_percent,
                    alert_minutes, project_name, updated_at
             FROM platform_settings"
        )
        .fetch_one(conn)
        .await
    }

    /// Lecture verrouillée, avant une modification.
    pub async fn lock(conn: &mut PgConnection) -> Result<PlatformSettings, sqlx::Error> {
        sqlx::query_as!(
            PlatformSettings,
            "SELECT default_memory_mb, max_memory_mb, default_cpu_percent, max_cpu_percent,
                    default_disk_mb, max_disk_mb, default_backup_limit, max_backup_limit,
                    audit_retention_days, mcp_enabled, sftp_enabled, file_history_days,
                    file_history_max_mb, alert_cpu_percent, alert_memory_percent,
                    alert_minutes, project_name, updated_at
             FROM platform_settings FOR UPDATE"
        )
        .fetch_one(conn)
        .await
    }

    /// Serveur MCP ouvert : lu à chaque requête, la bascule prend effet aussitôt.
    pub async fn mcp_enabled(conn: &mut PgConnection) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar!("SELECT mcp_enabled FROM platform_settings")
            .fetch_one(conn)
            .await
    }

    /// SFTP ouvert : lu à chaque connexion et à chaque bail, la bascule prend effet aussitôt.
    pub async fn sftp_enabled(conn: &mut PgConnection) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar!("SELECT sftp_enabled FROM platform_settings")
            .fetch_one(conn)
            .await
    }

    /// Rétention de l'historique des fichiers, envoyée aux nœuds.
    pub async fn file_history(conn: &mut PgConnection) -> Result<(i32, i32), sqlx::Error> {
        let row =
            sqlx::query!("SELECT file_history_days, file_history_max_mb FROM platform_settings")
                .fetch_one(conn)
                .await?;
        Ok((row.file_history_days, row.file_history_max_mb))
    }

    pub async fn save(
        conn: &mut PgConnection,
        settings: &PlatformSettings,
    ) -> Result<OffsetDateTime, sqlx::Error> {
        sqlx::query_scalar!(
            "UPDATE platform_settings SET
                default_memory_mb = $1, max_memory_mb = $2,
                default_cpu_percent = $3, max_cpu_percent = $4,
                default_disk_mb = $5, max_disk_mb = $6,
                default_backup_limit = $7, max_backup_limit = $8,
                audit_retention_days = $9, mcp_enabled = $10, alert_cpu_percent = $11,
                alert_memory_percent = $12, alert_minutes = $13, project_name = $14,
                sftp_enabled = $15, file_history_days = $16, file_history_max_mb = $17,
                updated_at = now()
             RETURNING updated_at",
            settings.default_memory_mb,
            settings.max_memory_mb,
            settings.default_cpu_percent,
            settings.max_cpu_percent,
            settings.default_disk_mb,
            settings.max_disk_mb,
            settings.default_backup_limit,
            settings.max_backup_limit,
            settings.audit_retention_days,
            settings.mcp_enabled,
            settings.alert_cpu_percent,
            settings.alert_memory_percent,
            settings.alert_minutes,
            settings.project_name.as_deref(),
            settings.sftp_enabled,
            settings.file_history_days,
            settings.file_history_max_mb,
        )
        .fetch_one(conn)
        .await
    }
}
