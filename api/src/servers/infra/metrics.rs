//! Persistance des stats historisées (ADR-0043) : minutes reçues des nœuds, cumul horaire,
//! lectures par plage et purge.

use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

/// Un point d'historique, à la minute ou à l'heure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricPoint {
    pub at: OffsetDateTime,
    pub samples: i32,
    pub cpu_millis_avg: i64,
    pub cpu_millis_max: i64,
    pub memory_bytes_avg: i64,
    pub memory_bytes_max: i64,
    pub memory_limit_bytes: i64,
    pub network_rx_bytes: i64,
    pub network_tx_bytes: i64,
}

pub struct MetricsRepository;

impl MetricsRepository {
    /// Enregistre une minute si le serveur est bien au nœud ; un renvoi écrase. `false` sinon.
    pub async fn upsert_if_owned(
        conn: &mut PgConnection,
        node_id: Uuid,
        server_id: Uuid,
        point: &MetricPoint,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query!(
            "INSERT INTO server_metrics (server_id, minute, samples, cpu_millis_avg, cpu_millis_max,
                                         memory_bytes_avg, memory_bytes_max, memory_limit_bytes,
                                         network_rx_bytes, network_tx_bytes)
             SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, $10
             WHERE EXISTS (SELECT 1 FROM servers WHERE id = $1 AND node_id = $11)
             ON CONFLICT (server_id, minute) DO UPDATE SET
                 samples = EXCLUDED.samples, cpu_millis_avg = EXCLUDED.cpu_millis_avg,
                 cpu_millis_max = EXCLUDED.cpu_millis_max, memory_bytes_avg = EXCLUDED.memory_bytes_avg,
                 memory_bytes_max = EXCLUDED.memory_bytes_max, memory_limit_bytes = EXCLUDED.memory_limit_bytes,
                 network_rx_bytes = EXCLUDED.network_rx_bytes, network_tx_bytes = EXCLUDED.network_tx_bytes",
            server_id,
            point.at,
            point.samples,
            point.cpu_millis_avg,
            point.cpu_millis_max,
            point.memory_bytes_avg,
            point.memory_bytes_max,
            point.memory_limit_bytes,
            point.network_rx_bytes,
            point.network_tx_bytes,
            node_id,
        )
        .execute(conn)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Minutes d'un serveur depuis `since`, dans l'ordre.
    pub async fn minutes(
        conn: &mut PgConnection,
        server_id: Uuid,
        since: OffsetDateTime,
    ) -> Result<Vec<MetricPoint>, sqlx::Error> {
        sqlx::query_as!(
            MetricPoint,
            r#"SELECT minute AS "at!", samples, cpu_millis_avg, cpu_millis_max, memory_bytes_avg,
                      memory_bytes_max, memory_limit_bytes, network_rx_bytes, network_tx_bytes
               FROM server_metrics WHERE server_id = $1 AND minute >= $2 ORDER BY minute"#,
            server_id,
            since
        )
        .fetch_all(conn)
        .await
    }

    /// Heures d'un serveur depuis `since`, dans l'ordre.
    pub async fn hours(
        conn: &mut PgConnection,
        server_id: Uuid,
        since: OffsetDateTime,
    ) -> Result<Vec<MetricPoint>, sqlx::Error> {
        sqlx::query_as!(
            MetricPoint,
            r#"SELECT hour AS "at!", samples, cpu_millis_avg, cpu_millis_max, memory_bytes_avg,
                      memory_bytes_max, memory_limit_bytes, network_rx_bytes, network_tx_bytes
               FROM server_metrics_hourly WHERE server_id = $1 AND hour >= $2 ORDER BY hour"#,
            server_id,
            since
        )
        .fetch_all(conn)
        .await
    }

    /// Cumule en heures toutes les minutes antérieures à `before` (moyennes pondérées par le
    /// nombre de mesures, pics, sommes réseau) ; une heure déjà cumulée est recalculée.
    pub async fn rollup(
        conn: &mut PgConnection,
        before: OffsetDateTime,
    ) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!(
            "INSERT INTO server_metrics_hourly (server_id, hour, samples, cpu_millis_avg, cpu_millis_max,
                                                memory_bytes_avg, memory_bytes_max, memory_limit_bytes,
                                                network_rx_bytes, network_tx_bytes)
             SELECT server_id, date_trunc('hour', minute), sum(samples)::integer,
                    (sum(cpu_millis_avg * samples) / greatest(sum(samples), 1))::bigint,
                    max(cpu_millis_max),
                    (sum(memory_bytes_avg * samples) / greatest(sum(samples), 1))::bigint,
                    max(memory_bytes_max), max(memory_limit_bytes),
                    sum(network_rx_bytes), sum(network_tx_bytes)
             FROM server_metrics WHERE minute < $1
             GROUP BY server_id, date_trunc('hour', minute)
             ON CONFLICT (server_id, hour) DO UPDATE SET
                 samples = EXCLUDED.samples, cpu_millis_avg = EXCLUDED.cpu_millis_avg,
                 cpu_millis_max = EXCLUDED.cpu_millis_max, memory_bytes_avg = EXCLUDED.memory_bytes_avg,
                 memory_bytes_max = EXCLUDED.memory_bytes_max, memory_limit_bytes = EXCLUDED.memory_limit_bytes,
                 network_rx_bytes = EXCLUDED.network_rx_bytes, network_tx_bytes = EXCLUDED.network_tx_bytes",
            before
        )
        .execute(conn)
        .await?;
        Ok(result.rows_affected())
    }

    /// Supprime les minutes antérieures à `before_minutes` et les heures antérieures à
    /// `before_hours` ; rend (minutes, heures) supprimées.
    pub async fn purge(
        conn: &mut PgConnection,
        before_minutes: OffsetDateTime,
        before_hours: OffsetDateTime,
    ) -> Result<(u64, u64), sqlx::Error> {
        let minutes = sqlx::query!(
            "DELETE FROM server_metrics WHERE minute < $1",
            before_minutes
        )
        .execute(&mut *conn)
        .await?
        .rows_affected();
        let hours = sqlx::query!(
            "DELETE FROM server_metrics_hourly WHERE hour < $1",
            before_hours
        )
        .execute(conn)
        .await?
        .rows_affected();
        Ok((minutes, hours))
    }
}
