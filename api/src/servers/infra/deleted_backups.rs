//! Sauvegardes gardées des serveurs supprimés (ADR-0059) : la trace en base des archives qui
//! attendent dans la corbeille de leur nœud, jusqu'à leur date d'effacement.

use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct DeletedBackupRecord {
    pub id: Uuid,
    pub server_id: Uuid,
    pub server_name: String,
    pub prefab_id: String,
    pub prefab_version: String,
    pub owner_ref: Option<String>,
    pub node_id: Uuid,
    pub node_name: String,
    pub name: String,
    pub size_bytes: Option<i64>,
    pub sha256: Option<String>,
    pub created_at: OffsetDateTime,
    pub deleted_at: OffsetDateTime,
    pub purge_at: OffsetDateTime,
}

#[derive(Debug)]
pub struct DeletedBackupRepository;

impl DeletedBackupRepository {
    /// Celles encore gardées (date d'effacement à venir), du serveur supprimé le plus récemment
    /// au plus ancien ; `server_id` : d'un seul serveur. Les expirées sont oubliées au passage :
    /// leur nœud les a effacées.
    pub async fn list(
        conn: &mut PgConnection,
        server_id: Option<Uuid>,
    ) -> Result<Vec<DeletedBackupRecord>, sqlx::Error> {
        sqlx::query!("DELETE FROM deleted_server_backups WHERE purge_at <= now()")
            .execute(&mut *conn)
            .await?;
        sqlx::query_as!(
            DeletedBackupRecord,
            "SELECT d.id, d.server_id, d.server_name, d.prefab_id, d.prefab_version, d.owner_ref,
                    d.node_id, n.name AS node_name, d.name, d.size_bytes, d.sha256, d.created_at,
                    d.deleted_at, d.purge_at
             FROM deleted_server_backups d JOIN nodes n ON n.id = d.node_id
             WHERE $1::uuid IS NULL OR d.server_id = $1
             ORDER BY d.deleted_at DESC, d.created_at DESC",
            server_id
        )
        .fetch_all(conn)
        .await
    }

    /// Reprises par `target` : deviennent ses sauvegardes prêtes, verrouillées pour que sa rotation
    /// ne les efface pas avant leur restauration ; quittent la liste.
    pub async fn adopt(
        conn: &mut PgConnection,
        from: Uuid,
        target: Uuid,
    ) -> Result<u64, sqlx::Error> {
        let adopted = sqlx::query!(
            "INSERT INTO backups (id, server_id, name, state, size_bytes, sha256, locked,
                                  created_at, completed_at)
             SELECT id, $2, name, 'ready', size_bytes, sha256, true, created_at, now()
             FROM deleted_server_backups WHERE server_id = $1",
            from,
            target
        )
        .execute(&mut *conn)
        .await?
        .rows_affected();
        sqlx::query!(
            "DELETE FROM deleted_server_backups WHERE server_id = $1",
            from
        )
        .execute(conn)
        .await?;
        Ok(adopted)
    }
}
