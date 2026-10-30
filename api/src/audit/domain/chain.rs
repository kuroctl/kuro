//! Chaînage du journal : `hash = SHA-256(prev_hash ‖ champs de l'événement)`.
//!
//! Chaque champ est préfixé par sa longueur (aucune ambiguïté de séparateur) et les détails
//! JSON sont sérialisés sous forme canonique (clés triées), pour que le hash recalculé depuis
//! la base (jsonb) soit identique à celui calculé à l'écriture.

use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;

/// `prev_hash` de la toute première ligne.
pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Contenu haché d'une ligne, tel que stocké.
#[derive(Debug, Clone)]
pub struct ChainRecord<'a> {
    pub id: Uuid,
    pub occurred_at: OffsetDateTime,
    pub actor_kind: &'a str,
    pub actor_id: Option<Uuid>,
    pub action: &'a str,
    pub outcome: &'a str,
    pub target_kind: Option<&'a str>,
    pub target_id: Option<&'a str>,
    pub ip: Option<&'a str>,
    pub user_agent: Option<&'a str>,
    pub request_id: Option<&'a str>,
    pub details: &'a Value,
}

impl ChainRecord<'_> {
    pub fn hash(&self, prev_hash: &str) -> String {
        let mut input = String::new();
        let micros = self.occurred_at.unix_timestamp_nanos() / 1_000;
        for field in [
            prev_hash,
            &self.id.to_string(),
            &micros.to_string(),
            self.actor_kind,
            &self.actor_id.map(|id| id.to_string()).unwrap_or_default(),
            self.action,
            self.outcome,
            self.target_kind.unwrap_or_default(),
            self.target_id.unwrap_or_default(),
            self.ip.unwrap_or_default(),
            self.user_agent.unwrap_or_default(),
            self.request_id.unwrap_or_default(),
            &canonical_json(self.details),
        ] {
            input.push_str(&field.len().to_string());
            input.push(':');
            input.push_str(field);
        }
        kuro_runtime::tls::sha256_hex(input.as_bytes())
    }
}

/// Horodatage à la précision de PostgreSQL (µs), pour un hash stable après relecture.
pub fn now() -> OffsetDateTime {
    let now = OffsetDateTime::now_utc();
    now.replace_nanosecond(now.nanosecond() / 1_000 * 1_000)
        .unwrap_or(now)
}

/// JSON à clés triées, indépendant de l'ordre d'insertion et du stockage jsonb.
fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let fields: Vec<String> = keys
                .into_iter()
                .map(|key| {
                    format!(
                        "{}:{}",
                        Value::String(key.clone()),
                        canonical_json(&map[key])
                    )
                })
                .collect();
            format!("{{{}}}", fields.join(","))
        }
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(canonical_json).collect();
            format!("[{}]", items.join(","))
        }
        other => other.to_string(),
    }
}
