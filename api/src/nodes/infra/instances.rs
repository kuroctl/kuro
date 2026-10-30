//! Instances de l'API en base (ADR-0037) : inscription, battement, nettoyage des instances
//! mortes et de ce qu'elles tenaient.

use std::time::Duration;

use sqlx::PgPool;

use crate::nodes::domain::InstanceId;

/// Ce qu'un balayage a nettoyé.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Swept {
    /// Instances mortes supprimées.
    pub instances: u64,
    /// Nœuds passés `offline` : tenus par une instance morte, ou par aucune.
    pub nodes: u64,
}

#[derive(Debug, Clone)]
pub struct InstanceRepository {
    pool: PgPool,
}

impl InstanceRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn register(&self, id: InstanceId) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO api_instances (id) VALUES ($1)
             ON CONFLICT (id) DO UPDATE SET last_seen_at = now()",
            id.as_uuid()
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// `false` si la ligne a disparu : une autre instance nous a crue morte (pause longue,
    /// horloge) ; il faut se réinscrire.
    pub async fn heartbeat(&self, id: InstanceId) -> Result<bool, sqlx::Error> {
        let result = sqlx::query!(
            "UPDATE api_instances SET last_seen_at = now() WHERE id = $1",
            id.as_uuid()
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Arrêt propre : les nœuds tenus repassent `offline` tout de suite, la ligne disparaît.
    pub async fn unregister(&self, id: InstanceId) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        sqlx::query!(
            "UPDATE nodes SET status = 'offline', instance_id = NULL, updated_at = now()
             WHERE instance_id = $1",
            id.as_uuid()
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!("DELETE FROM api_instances WHERE id = $1", id.as_uuid())
            .execute(&mut *tx)
            .await?;
        tx.commit().await
    }

    /// Instances (autres que `me`) sans battement depuis `dead_after` : leurs nœuds passent
    /// `offline`, ainsi que tout nœud `online` que personne ne tient ; leurs lignes et les
    /// commandes déposées depuis plus d'une minute disparaissent.
    pub async fn sweep(&self, me: InstanceId, dead_after: Duration) -> Result<Swept, sqlx::Error> {
        let me = me.as_uuid();
        let dead_after = dead_after.as_secs_f64();
        let mut tx = self.pool.begin().await?;
        let nodes = sqlx::query!(
            "UPDATE nodes SET status = 'offline', instance_id = NULL, updated_at = now()
             WHERE status = 'online'
               AND (instance_id IS NULL
                    OR instance_id IN (SELECT id FROM api_instances
                                       WHERE id <> $1
                                         AND last_seen_at < now() - $2 * interval '1 second'))",
            me,
            dead_after
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        let instances = sqlx::query!(
            "DELETE FROM api_instances
             WHERE id <> $1 AND last_seen_at < now() - $2 * interval '1 second'",
            me,
            dead_after
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        sqlx::query!("DELETE FROM node_commands WHERE created_at < now() - interval '1 minute'")
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Swept { instances, nodes })
    }
}
