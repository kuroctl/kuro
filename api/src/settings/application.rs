//! Lecture et modification des paramètres du projet ; plafonds des ressources des serveurs.

use std::ops::RangeInclusive;

use serde_json::{Map, Value, json};
use sqlx::PgPool;

use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::error_code::{ErrorCode, FieldError};
use crate::servers::application::{BACKUP_LIMIT, CPU_PERCENT, DISK_MB, MEMORY_MB};
use crate::settings::infra::SettingsRepository;

pub use crate::settings::infra::PlatformSettings;

/// Conservation du journal d'audit : 30 jours au moins, pour ne pas le vider par erreur.
pub const RETENTION_DAYS: RangeInclusive<i32> = 30..=3650;
/// Seuils d'alerte des stats (ADR-0043) : en % de l'allocation, et durée en minutes.
pub const ALERT_PERCENT: RangeInclusive<i32> = 50..=100;
pub const ALERT_MINUTES: RangeInclusive<i32> = 1..=60;

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("invalid settings")]
    Invalid(Vec<FieldError>),
    #[error("unprocessable: {}", .0.as_str())]
    Unprocessable(ErrorCode),
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<sqlx::Error> for SettingsError {
    fn from(error: sqlx::Error) -> Self {
        Self::Internal(error.to_string())
    }
}

/// Modification partielle. Plafond : absent → inchangé, `None` → supprimé.
#[derive(Debug, Clone, Default)]
pub struct SettingsPatch {
    pub default_memory_mb: Option<i32>,
    pub max_memory_mb: Option<Option<i32>>,
    pub default_cpu_percent: Option<i32>,
    pub max_cpu_percent: Option<Option<i32>>,
    pub default_disk_mb: Option<i32>,
    pub max_disk_mb: Option<Option<i32>>,
    pub default_backup_limit: Option<i32>,
    pub max_backup_limit: Option<Option<i32>>,
    pub audit_retention_days: Option<i32>,
    pub mcp_enabled: Option<bool>,
    pub alert_cpu_percent: Option<i32>,
    pub alert_memory_percent: Option<i32>,
    pub alert_minutes: Option<i32>,
    /// `None` : inchangé ; `Some(None)` : retiré (« Kuro »).
    pub project_name: Option<Option<String>>,
}

pub async fn get(pool: &PgPool) -> Result<PlatformSettings, SettingsError> {
    let mut conn = pool.acquire().await?;
    Ok(SettingsRepository::get(&mut conn).await?)
}

/// Applique la modification, vérifie chaque borne, et journalise l'ancienne et la nouvelle
/// valeur de chaque champ modifié (aucune n'est un secret).
pub async fn update(
    pool: &PgPool,
    context: &AuditContext,
    patch: SettingsPatch,
) -> Result<PlatformSettings, SettingsError> {
    let mut tx = AuditedTx::begin(pool, context).await?;
    let current = SettingsRepository::lock(&mut tx).await?;
    let mut next = current.clone();
    macro_rules! apply {
        ($($field:ident),*) => {
            $(if let Some(value) = patch.$field { next.$field = value; })*
        };
    }
    apply!(
        default_memory_mb,
        max_memory_mb,
        default_cpu_percent,
        max_cpu_percent,
        default_disk_mb,
        max_disk_mb,
        default_backup_limit,
        max_backup_limit,
        audit_retention_days,
        mcp_enabled,
        alert_cpu_percent,
        alert_memory_percent,
        alert_minutes,
        project_name
    );
    next.project_name = next
        .project_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned);
    validate(&next)?;

    let mut changed = Map::new();
    let mut record = |field: &str, from: Value, to: Value| {
        if from != to {
            changed.insert(field.to_owned(), json!({ "from": from, "to": to }));
        }
    };
    record(
        "default_memory_mb",
        json!(current.default_memory_mb),
        json!(next.default_memory_mb),
    );
    record(
        "max_memory_mb",
        json!(current.max_memory_mb),
        json!(next.max_memory_mb),
    );
    record(
        "default_cpu_percent",
        json!(current.default_cpu_percent),
        json!(next.default_cpu_percent),
    );
    record(
        "max_cpu_percent",
        json!(current.max_cpu_percent),
        json!(next.max_cpu_percent),
    );
    record(
        "default_disk_mb",
        json!(current.default_disk_mb),
        json!(next.default_disk_mb),
    );
    record(
        "max_disk_mb",
        json!(current.max_disk_mb),
        json!(next.max_disk_mb),
    );
    record(
        "default_backup_limit",
        json!(current.default_backup_limit),
        json!(next.default_backup_limit),
    );
    record(
        "max_backup_limit",
        json!(current.max_backup_limit),
        json!(next.max_backup_limit),
    );
    record(
        "audit_retention_days",
        json!(current.audit_retention_days),
        json!(next.audit_retention_days),
    );
    record(
        "mcp_enabled",
        json!(current.mcp_enabled),
        json!(next.mcp_enabled),
    );
    record(
        "alert_cpu_percent",
        json!(current.alert_cpu_percent),
        json!(next.alert_cpu_percent),
    );
    record(
        "alert_memory_percent",
        json!(current.alert_memory_percent),
        json!(next.alert_memory_percent),
    );
    record(
        "alert_minutes",
        json!(current.alert_minutes),
        json!(next.alert_minutes),
    );
    record(
        "project_name",
        json!(current.project_name),
        json!(next.project_name),
    );
    if changed.is_empty() {
        return Err(SettingsError::Unprocessable(ErrorCode::EmptyPatch));
    }

    next.updated_at = SettingsRepository::save(&mut tx, &next).await?;
    tx.commit(
        AuditEvent::success(AuditAction::SettingsUpdated).details(json!({ "changed": changed })),
    )
    .await?;
    Ok(next)
}

fn validate(settings: &PlatformSettings) -> Result<(), SettingsError> {
    let mut errors = Vec::new();
    let resources = [
        (
            "memory_mb",
            settings.default_memory_mb,
            settings.max_memory_mb,
            MEMORY_MB,
        ),
        (
            "cpu_percent",
            settings.default_cpu_percent,
            settings.max_cpu_percent,
            CPU_PERCENT,
        ),
        (
            "disk_mb",
            settings.default_disk_mb,
            settings.max_disk_mb,
            DISK_MB,
        ),
        (
            "backup_limit",
            settings.default_backup_limit,
            settings.max_backup_limit,
            BACKUP_LIMIT,
        ),
    ];
    for (name, default, max, range) in resources {
        if !range.contains(&default) {
            errors.push(out_of_range(format!("default_{name}"), &range));
        }
        // Un plafond disque de 0 n'aurait pas de sens : 0 veut dire illimité.
        let floor = if name == "disk_mb" { 1 } else { *range.start() };
        if let Some(max) = max
            && !(floor..=*range.end()).contains(&max)
        {
            errors.push(out_of_range(format!("max_{name}"), &(floor..=*range.end())));
        }
        if let Some(error) = above_ceiling(name, default, max) {
            errors.push(FieldError {
                field: format!("default_{name}").into(),
                ..error
            });
        }
    }
    if !RETENTION_DAYS.contains(&settings.audit_retention_days) {
        errors.push(out_of_range(
            "audit_retention_days".to_owned(),
            &RETENTION_DAYS,
        ));
    }
    for (field, value) in [
        ("alert_cpu_percent", settings.alert_cpu_percent),
        ("alert_memory_percent", settings.alert_memory_percent),
    ] {
        if !ALERT_PERCENT.contains(&value) {
            errors.push(out_of_range(field.to_owned(), &ALERT_PERCENT));
        }
    }
    if !ALERT_MINUTES.contains(&settings.alert_minutes) {
        errors.push(out_of_range("alert_minutes".to_owned(), &ALERT_MINUTES));
    }
    if let Some(name) = &settings.project_name
        && (name.chars().count() > 80 || name.chars().any(char::is_control))
    {
        errors.push(FieldError::new(
            "project_name",
            ErrorCode::InvalidProjectName,
        ));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(SettingsError::Invalid(errors))
    }
}

fn out_of_range(field: String, range: &RangeInclusive<i32>) -> FieldError {
    FieldError::new(field, ErrorCode::ValueOutOfRange)
        .param("min", *range.start())
        .param("max", *range.end())
}

/// Valeur au-delà du plafond, sous le champ `field`. Disque `0` (illimité) dépasse tout plafond.
fn above_ceiling(field: &'static str, value: i32, max: Option<i32>) -> Option<FieldError> {
    let max = max?;
    let unlimited = field == "disk_mb" && value == 0;
    (unlimited || value > max)
        .then(|| FieldError::new(field, ErrorCode::ResourceAboveCeiling).param("max", max))
}

/// Ressource sous le minimum de son prefab (ADR-0031), sous le champ de la ressource. Seules les
/// valeurs `changed` sont vérifiées : un serveur existant sous le minimum continue de tourner.
/// Disque `0` (illimité) satisfait tout minimum.
pub fn check_minimums(
    needs: &kuro_prefab::manifest::ResourceNeeds,
    resources: Resources,
    changed: impl Fn(&str) -> bool,
) -> Result<(), FieldError> {
    let values = [
        ("memory_mb", resources.memory_mb),
        ("cpu_percent", resources.cpu_percent),
        ("disk_mb", resources.disk_mb),
    ];
    for (field, value) in values {
        let Some(min) = needs.min(field) else {
            continue;
        };
        let unlimited = field == "disk_mb" && value == 0;
        if changed(field) && !unlimited && i64::from(value) < i64::from(min) {
            return Err(
                FieldError::new(field, ErrorCode::ResourceBelowPrefabMinimum).param("min", min),
            );
        }
    }
    Ok(())
}

/// Ressources demandées pour un serveur.
#[derive(Debug, Clone, Copy)]
pub struct Resources {
    pub memory_mb: i32,
    pub cpu_percent: i32,
    pub disk_mb: i32,
    pub backup_limit: i32,
}

impl PlatformSettings {
    /// Premier plafond dépassé, sous le champ de la ressource. Seules les valeurs `changed`
    /// sont vérifiées : baisser un plafond ne bloque pas un serveur déjà au-dessus.
    pub fn check_ceilings(
        &self,
        resources: Resources,
        changed: impl Fn(&str) -> bool,
    ) -> Result<(), FieldError> {
        let checks = [
            ("memory_mb", resources.memory_mb, self.max_memory_mb),
            ("cpu_percent", resources.cpu_percent, self.max_cpu_percent),
            ("disk_mb", resources.disk_mb, self.max_disk_mb),
            (
                "backup_limit",
                resources.backup_limit,
                self.max_backup_limit,
            ),
        ];
        for (field, value, max) in checks {
            if changed(field)
                && let Some(error) = above_ceiling(field, value, max)
            {
                return Err(error);
            }
        }
        Ok(())
    }
}
