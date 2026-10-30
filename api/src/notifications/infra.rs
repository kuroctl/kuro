//! Persistance des notifications (ADR-0044) : canaux, règles et journal des envois.

use serde_json::Value;
use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ChannelRecord {
    pub id: Uuid,
    pub name: String,
    pub kind: String,
    /// Configuration chiffrée (`ChannelConfig` en JSON).
    pub config: Vec<u8>,
    pub url_hint: String,
    pub language: String,
    pub enabled: bool,
    /// Serveur propriétaire (canal créé depuis sa page) ; absent : canal partagé.
    pub server_id: Option<Uuid>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

pub struct ChannelFields<'a> {
    pub name: &'a str,
    pub kind: &'a str,
    pub config: &'a [u8],
    pub url_hint: &'a str,
    pub language: &'a str,
    pub enabled: bool,
    pub server_id: Option<Uuid>,
}

#[derive(Debug, Clone)]
pub struct RuleRecord {
    pub id: Uuid,
    pub name: String,
    pub scope: String,
    pub node_id: Option<Uuid>,
    pub server_id: Option<Uuid>,
    pub events: Vec<String>,
    pub channels: Vec<Uuid>,
    pub enabled: bool,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

pub struct RuleFields<'a> {
    pub name: &'a str,
    pub scope: &'a str,
    pub node_id: Option<Uuid>,
    pub server_id: Option<Uuid>,
    pub events: &'a [String],
    pub channels: &'a [Uuid],
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct DeliveryRecord {
    pub id: Uuid,
    pub rule_id: Option<Uuid>,
    pub channel_id: Uuid,
    pub event: Value,
    pub status: String,
    pub attempts: i32,
    pub next_attempt_at: OffsetDateTime,
    pub last_error: Option<String>,
    pub created_at: OffsetDateTime,
    pub sent_at: Option<OffsetDateTime>,
}

pub struct NotificationRepository;

impl NotificationRepository {
    /// Canaux partagés (ceux des paramètres) ; les canaux propres à un serveur n'y sont pas.
    pub async fn channels(conn: &mut PgConnection) -> Result<Vec<ChannelRecord>, sqlx::Error> {
        sqlx::query_as!(
            ChannelRecord,
            "SELECT id, name, kind, config, url_hint, language, enabled, server_id, created_at, updated_at
             FROM notification_channels WHERE server_id IS NULL ORDER BY name, created_at"
        )
        .fetch_all(conn)
        .await
    }

    pub async fn channel(
        conn: &mut PgConnection,
        id: Uuid,
    ) -> Result<Option<ChannelRecord>, sqlx::Error> {
        sqlx::query_as!(
            ChannelRecord,
            "SELECT id, name, kind, config, url_hint, language, enabled, server_id, created_at, updated_at
             FROM notification_channels WHERE id = $1",
            id
        )
        .fetch_optional(conn)
        .await
    }

    pub async fn insert_channel(
        conn: &mut PgConnection,
        id: Uuid,
        created_by: Option<Uuid>,
        fields: &ChannelFields<'_>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO notification_channels (id, name, kind, config, url_hint, language, enabled, created_by, server_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            id,
            fields.name,
            fields.kind,
            fields.config,
            fields.url_hint,
            fields.language,
            fields.enabled,
            created_by,
            fields.server_id,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn update_channel(
        conn: &mut PgConnection,
        id: Uuid,
        fields: &ChannelFields<'_>,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query!(
            "UPDATE notification_channels
             SET name = $2, kind = $3, config = $4, url_hint = $5, language = $6, enabled = $7,
                 updated_at = now()
             WHERE id = $1",
            id,
            fields.name,
            fields.kind,
            fields.config,
            fields.url_hint,
            fields.language,
            fields.enabled,
        )
        .execute(conn)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Supprime le canal ; les règles qui ne visaient que lui partent, les autres l'oublient.
    pub async fn delete_channel(conn: &mut PgConnection, id: Uuid) -> Result<bool, sqlx::Error> {
        sqlx::query!(
            "DELETE FROM notification_rules WHERE channels = ARRAY[$1]::uuid[]",
            id
        )
        .execute(&mut *conn)
        .await?;
        sqlx::query!(
            "UPDATE notification_rules SET channels = array_remove(channels, $1), updated_at = now()
             WHERE $1 = ANY(channels)",
            id
        )
        .execute(&mut *conn)
        .await?;
        let result = sqlx::query!("DELETE FROM notification_channels WHERE id = $1", id)
            .execute(conn)
            .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Règles des paramètres : celles qui ne visent aucun canal propre à un serveur (les
    /// abonnements d'un serveur se lisent depuis sa page).
    pub async fn rules(conn: &mut PgConnection) -> Result<Vec<RuleRecord>, sqlx::Error> {
        sqlx::query_as!(
            RuleRecord,
            "SELECT r.id, r.name, r.scope, r.node_id, r.server_id, r.events, r.channels, r.enabled,
                    r.created_at, r.updated_at
             FROM notification_rules r
             WHERE NOT EXISTS (SELECT 1 FROM notification_channels c
                               WHERE c.id = ANY(r.channels) AND c.server_id IS NOT NULL)
             ORDER BY r.name, r.created_at"
        )
        .fetch_all(conn)
        .await
    }

    /// Abonnements d'un serveur : ses règles de portée serveur.
    pub async fn rules_of_server(
        conn: &mut PgConnection,
        server_id: Uuid,
    ) -> Result<Vec<RuleRecord>, sqlx::Error> {
        sqlx::query_as!(
            RuleRecord,
            "SELECT id, name, scope, node_id, server_id, events, channels, enabled, created_at, updated_at
             FROM notification_rules WHERE scope = 'server' AND server_id = $1
             ORDER BY created_at",
            server_id
        )
        .fetch_all(conn)
        .await
    }

    /// Envois dont l'événement concerne ce serveur, les plus récents d'abord.
    pub async fn deliveries_of_server(
        conn: &mut PgConnection,
        server_id: Uuid,
        limit: i64,
    ) -> Result<Vec<DeliveryRecord>, sqlx::Error> {
        sqlx::query_as!(
            DeliveryRecord,
            "SELECT id, rule_id, channel_id, event, status, attempts, next_attempt_at, last_error,
                    created_at, sent_at
             FROM notification_deliveries WHERE event -> 'server' ->> 'id' = $1
             ORDER BY created_at DESC LIMIT $2",
            server_id.to_string(),
            limit
        )
        .fetch_all(conn)
        .await
    }

    pub async fn rule(
        conn: &mut PgConnection,
        id: Uuid,
    ) -> Result<Option<RuleRecord>, sqlx::Error> {
        sqlx::query_as!(
            RuleRecord,
            "SELECT id, name, scope, node_id, server_id, events, channels, enabled, created_at, updated_at
             FROM notification_rules WHERE id = $1",
            id
        )
        .fetch_optional(conn)
        .await
    }

    /// Règles activées dont au moins un canal est activé, pour faire correspondre un événement.
    pub async fn enabled_rules(conn: &mut PgConnection) -> Result<Vec<RuleRecord>, sqlx::Error> {
        sqlx::query_as!(
            RuleRecord,
            "SELECT id, name, scope, node_id, server_id, events, channels, enabled, created_at, updated_at
             FROM notification_rules WHERE enabled ORDER BY created_at"
        )
        .fetch_all(conn)
        .await
    }

    /// Canaux activés parmi `ids`.
    pub async fn enabled_channels(
        conn: &mut PgConnection,
        ids: &[Uuid],
    ) -> Result<Vec<Uuid>, sqlx::Error> {
        let rows = sqlx::query!(
            "SELECT id FROM notification_channels WHERE enabled AND id = ANY($1)",
            ids
        )
        .fetch_all(conn)
        .await?;
        Ok(rows.into_iter().map(|row| row.id).collect())
    }

    pub async fn existing_channels(
        conn: &mut PgConnection,
        ids: &[Uuid],
    ) -> Result<Vec<Uuid>, sqlx::Error> {
        let rows = sqlx::query!(
            "SELECT id FROM notification_channels WHERE id = ANY($1)",
            ids
        )
        .fetch_all(conn)
        .await?;
        Ok(rows.into_iter().map(|row| row.id).collect())
    }

    pub async fn insert_rule(
        conn: &mut PgConnection,
        id: Uuid,
        created_by: Option<Uuid>,
        fields: &RuleFields<'_>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO notification_rules (id, name, scope, node_id, server_id, events, channels, enabled, created_by)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            id,
            fields.name,
            fields.scope,
            fields.node_id,
            fields.server_id,
            fields.events,
            fields.channels,
            fields.enabled,
            created_by,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn update_rule(
        conn: &mut PgConnection,
        id: Uuid,
        fields: &RuleFields<'_>,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query!(
            "UPDATE notification_rules
             SET name = $2, scope = $3, node_id = $4, server_id = $5, events = $6, channels = $7,
                 enabled = $8, updated_at = now()
             WHERE id = $1",
            id,
            fields.name,
            fields.scope,
            fields.node_id,
            fields.server_id,
            fields.events,
            fields.channels,
            fields.enabled,
        )
        .execute(conn)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn delete_rule(conn: &mut PgConnection, id: Uuid) -> Result<bool, sqlx::Error> {
        let result = sqlx::query!("DELETE FROM notification_rules WHERE id = $1", id)
            .execute(conn)
            .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn deliveries(
        conn: &mut PgConnection,
        limit: i64,
    ) -> Result<Vec<DeliveryRecord>, sqlx::Error> {
        sqlx::query_as!(
            DeliveryRecord,
            "SELECT id, rule_id, channel_id, event, status, attempts, next_attempt_at, last_error,
                    created_at, sent_at
             FROM notification_deliveries ORDER BY created_at DESC LIMIT $1",
            limit
        )
        .fetch_all(conn)
        .await
    }

    pub async fn insert_delivery(
        conn: &mut PgConnection,
        id: Uuid,
        rule_id: Uuid,
        channel_id: Uuid,
        event: &Value,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO notification_deliveries (id, rule_id, channel_id, event, status)
             VALUES ($1, $2, $3, $4, 'pending')",
            id,
            rule_id,
            channel_id,
            event,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Réserve les envois dus pour deux minutes (bail) : une autre instance ne les prend pas
    /// pendant ce temps, et un processus arrêté en plein envoi les laisse revenir.
    pub async fn claim_due(
        conn: &mut PgConnection,
        limit: i64,
    ) -> Result<Vec<DeliveryRecord>, sqlx::Error> {
        sqlx::query_as!(
            DeliveryRecord,
            "UPDATE notification_deliveries
             SET next_attempt_at = now() + interval '2 minutes'
             WHERE id IN (SELECT id FROM notification_deliveries
                          WHERE status = 'pending' AND next_attempt_at <= now()
                          ORDER BY next_attempt_at
                          FOR UPDATE SKIP LOCKED
                          LIMIT $1)
             RETURNING id, rule_id, channel_id, event, status, attempts, next_attempt_at, last_error,
                       created_at, sent_at",
            limit
        )
        .fetch_all(conn)
        .await
    }

    /// Envois réussis vers ce canal pendant la dernière minute.
    pub async fn sent_last_minute(
        conn: &mut PgConnection,
        channel_id: Uuid,
    ) -> Result<i64, sqlx::Error> {
        let row = sqlx::query!(
            r#"SELECT count(*) AS "count!" FROM notification_deliveries
               WHERE channel_id = $1 AND status = 'sent' AND sent_at > now() - interval '1 minute'"#,
            channel_id
        )
        .fetch_one(conn)
        .await?;
        Ok(row.count)
    }

    pub async fn mark_sent(
        conn: &mut PgConnection,
        id: Uuid,
        attempts: i32,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE notification_deliveries
             SET status = 'sent', attempts = $2, sent_at = now(), last_error = NULL WHERE id = $1",
            id,
            attempts
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn mark_retry(
        conn: &mut PgConnection,
        id: Uuid,
        attempts: i32,
        next_attempt_at: OffsetDateTime,
        error: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE notification_deliveries
             SET attempts = $2, next_attempt_at = $3, last_error = $4 WHERE id = $1",
            id,
            attempts,
            next_attempt_at,
            error
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn mark_ended(
        conn: &mut PgConnection,
        id: Uuid,
        status: &str,
        attempts: i32,
        error: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE notification_deliveries SET status = $2, attempts = $3, last_error = $4 WHERE id = $1",
            id,
            status,
            attempts,
            error
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Repousse un envoi sans compter de tentative (limitation de débit).
    pub async fn postpone(
        conn: &mut PgConnection,
        id: Uuid,
        next_attempt_at: OffsetDateTime,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE notification_deliveries SET next_attempt_at = $2 WHERE id = $1",
            id,
            next_attempt_at
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Envois plus vieux que `days` jours.
    pub async fn purge(conn: &mut PgConnection, days: i32) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!(
            "DELETE FROM notification_deliveries WHERE created_at < now() - $1 * interval '1 day'",
            f64::from(days)
        )
        .execute(conn)
        .await?;
        Ok(result.rows_affected())
    }
}
