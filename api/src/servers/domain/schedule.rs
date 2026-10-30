//! Tâches planifiées (ADR-0022) : déclencheur cron lu dans un fuseau horaire, étapes exécutées
//! dans l'ordre.

use std::str::FromStr;

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use croner::Cron;
use croner::parser::{CronParser, Seconds};
use kuro_protocol::api::server::PowerAction;
use time::OffsetDateTime;

/// Tâches par serveur, étapes par tâche, exécutions gardées par tâche.
pub const MAX_SCHEDULES_PER_SERVER: i64 = 20;
pub const MAX_STEPS: usize = 10;
pub const RUNS_KEPT: i64 = 20;
/// Attente maximale avant une étape.
pub const MAX_DELAY_SECS: i32 = 3600;
pub const MAX_NAME_CHARS: usize = 64;

/// Déclencheur : expression à 5 champs, lue dans le fuseau donné (heure d'été comprise).
#[derive(Debug, Clone)]
pub struct Trigger {
    cron: Cron,
    timezone: Tz,
}

/// Raison du refus d'un déclencheur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerError {
    Cron,
    Timezone,
}

impl Trigger {
    pub fn parse(cron: &str, timezone: &str) -> Result<Self, TriggerError> {
        // Secondes interdites : 5 champs exactement, comme le cron d'Unix.
        let cron = CronParser::builder()
            .seconds(Seconds::Disallowed)
            .build()
            .parse(cron)
            .map_err(|_| TriggerError::Cron)?;
        let timezone = Tz::from_str(timezone).map_err(|_| TriggerError::Timezone)?;
        Ok(Self { cron, timezone })
    }

    /// Prochaine échéance strictement après `after`. `None` : l'expression n'arrive jamais
    /// (31 février…).
    pub fn next_after(&self, after: OffsetDateTime) -> Option<OffsetDateTime> {
        let utc = DateTime::<Utc>::from_timestamp(after.unix_timestamp(), after.nanosecond())?;
        let local = utc.with_timezone(&self.timezone);
        let next = self.cron.find_next_occurrence(&local, false).ok()?;
        OffsetDateTime::from_unix_timestamp(next.timestamp()).ok()
    }

    /// Les `count` prochaines échéances après `after`.
    pub fn upcoming(&self, after: OffsetDateTime, count: usize) -> Vec<OffsetDateTime> {
        let mut runs = Vec::with_capacity(count);
        let mut cursor = after;
        while runs.len() < count {
            let Some(next) = self.next_after(cursor) else {
                break;
            };
            runs.push(next);
            cursor = next;
        }
        runs
    }
}

/// Étape d'une tâche.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepAction {
    /// Ligne envoyée à la console.
    Command(String),
    Power(PowerAction),
    /// Sauvegarde, nommée ou datée ; l'étape attend sa fin.
    Backup(Option<String>),
}

impl StepAction {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Command(_) => "command",
            Self::Power(_) => "power",
            Self::Backup(_) => "backup",
        }
    }

    /// Valeur stockée dans `payload`.
    pub fn payload(&self) -> Option<String> {
        match self {
            Self::Command(line) => Some(line.clone()),
            Self::Power(action) => Some(power_key(*action).to_owned()),
            Self::Backup(name) => name.clone(),
        }
    }

    /// Relit une étape stockée ; `None` si la ligne est incohérente.
    pub fn from_stored(kind: &str, payload: Option<&str>) -> Option<Self> {
        match (kind, payload) {
            ("command", Some(line)) => Some(Self::Command(line.to_owned())),
            ("power", Some(action)) => power_from_key(action).map(Self::Power),
            ("backup", name) => Some(Self::Backup(name.map(str::to_owned))),
            _ => None,
        }
    }
}

pub fn power_key(action: PowerAction) -> &'static str {
    match action {
        PowerAction::Start => "start",
        PowerAction::Stop => "stop",
        PowerAction::Restart => "restart",
        PowerAction::Kill => "kill",
    }
}

pub fn power_from_key(key: &str) -> Option<PowerAction> {
    match key {
        "start" => Some(PowerAction::Start),
        "stop" => Some(PowerAction::Stop),
        "restart" => Some(PowerAction::Restart),
        "kill" => Some(PowerAction::Kill),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub action: StepAction,
    pub delay_secs: i32,
    pub continue_on_failure: bool,
}

#[cfg(test)]
mod tests {
    use time::macros::datetime;

    use super::*;

    #[test]
    fn five_fields_only() {
        assert!(Trigger::parse("0 4 * * *", "UTC").is_ok());
        assert_eq!(
            Trigger::parse("0 0 4 * * *", "UTC").unwrap_err(),
            TriggerError::Cron
        );
        assert_eq!(
            Trigger::parse("0 4 * * *", "Mars/Olympus").unwrap_err(),
            TriggerError::Timezone
        );
    }

    #[test]
    fn reads_the_expression_in_its_timezone() {
        let trigger = Trigger::parse("0 4 * * *", "Europe/Paris").unwrap();
        // Hiver : UTC+1, 04:00 à Paris = 03:00 UTC.
        assert_eq!(
            trigger.next_after(datetime!(2026-12-01 12:00 UTC)),
            Some(datetime!(2026-12-02 03:00 UTC))
        );
        // Été : UTC+2.
        assert_eq!(
            trigger.next_after(datetime!(2026-07-01 12:00 UTC)),
            Some(datetime!(2026-07-02 02:00 UTC))
        );
    }

    #[test]
    fn follows_the_switch_to_summer_time() {
        // 29 mars 2026 : 02:00 n'existe pas à Paris ; 03:30 a bien lieu, à 01:30 UTC.
        let trigger = Trigger::parse("30 3 * * *", "Europe/Paris").unwrap();
        assert_eq!(
            trigger.next_after(datetime!(2026-03-28 12:00 UTC)),
            Some(datetime!(2026-03-29 01:30 UTC))
        );
    }

    #[test]
    fn lists_upcoming_runs() {
        let trigger = Trigger::parse("0 */6 * * *", "UTC").unwrap();
        assert_eq!(
            trigger.upcoming(datetime!(2026-10-28 05:00 UTC), 3),
            vec![
                datetime!(2026-10-28 06:00 UTC),
                datetime!(2026-10-28 12:00 UTC),
                datetime!(2026-10-28 18:00 UTC),
            ]
        );
    }

    #[test]
    fn steps_round_trip_through_storage() {
        for action in [
            StepAction::Command("say hi".to_owned()),
            StepAction::Power(PowerAction::Restart),
            StepAction::Backup(None),
            StepAction::Backup(Some("nuit".to_owned())),
        ] {
            let payload = action.payload();
            assert_eq!(
                StepAction::from_stored(action.kind(), payload.as_deref()),
                Some(action)
            );
        }
        assert_eq!(StepAction::from_stored("power", Some("explode")), None);
    }
}
