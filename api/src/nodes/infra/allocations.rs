use std::collections::HashMap;

use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

/// Port d'un nœud, libre ou assigné à un serveur.
#[derive(Debug, Clone)]
pub struct AllocationRecord {
    pub id: Uuid,
    pub node_id: Uuid,
    pub ip: String,
    pub port: i32,
    pub server_id: Option<Uuid>,
    pub server_name: Option<String>,
    /// Port du prefab servi (`game`, `http`…).
    pub port_name: Option<String>,
    pub created_at: OffsetDateTime,
}

#[derive(Debug)]
pub struct AllocationRepository;

impl AllocationRepository {
    pub async fn list(
        conn: &mut PgConnection,
        node_id: Uuid,
    ) -> Result<Vec<AllocationRecord>, sqlx::Error> {
        sqlx::query_as!(
            AllocationRecord,
            r#"SELECT a.id, a.node_id, a.ip, a.port, a.server_id,
                      s.name AS "server_name?", a.port_name, a.created_at
               FROM allocations a LEFT JOIN servers s ON s.id = a.server_id
               WHERE a.node_id = $1
               ORDER BY a.ip, a.port"#,
            node_id,
        )
        .fetch_all(conn)
        .await
    }

    pub async fn get(
        conn: &mut PgConnection,
        node_id: Uuid,
        id: Uuid,
    ) -> Result<Option<AllocationRecord>, sqlx::Error> {
        sqlx::query_as!(
            AllocationRecord,
            r#"SELECT a.id, a.node_id, a.ip, a.port, a.server_id,
                      s.name AS "server_name?", a.port_name, a.created_at
               FROM allocations a LEFT JOIN servers s ON s.id = a.server_id
               WHERE a.node_id = $1 AND a.id = $2"#,
            node_id,
            id,
        )
        .fetch_optional(conn)
        .await
    }

    /// Ajoute les ports absents ; retourne le nombre créé (les existants sont ignorés).
    pub async fn insert_many(
        conn: &mut PgConnection,
        node_id: Uuid,
        ip: &str,
        ports: &[i32],
    ) -> Result<u64, sqlx::Error> {
        let ids: Vec<Uuid> = ports.iter().map(|_| Uuid::now_v7()).collect();
        let created = sqlx::query!(
            "INSERT INTO allocations (id, node_id, ip, port)
             SELECT id, $2, $3, port FROM UNNEST($1::uuid[], $4::int[]) AS t(id, port)
             ON CONFLICT (node_id, ip, port) DO NOTHING",
            &ids,
            node_id,
            ip,
            ports,
        )
        .execute(conn)
        .await?;
        Ok(created.rows_affected())
    }

    /// Supprime une allocation libre ; `false` si elle est assignée ou absente.
    pub async fn delete_free(conn: &mut PgConnection, id: Uuid) -> Result<bool, sqlx::Error> {
        let deleted = sqlx::query!(
            "DELETE FROM allocations WHERE id = $1 AND server_id IS NULL",
            id
        )
        .execute(conn)
        .await?;
        Ok(deleted.rows_affected() == 1)
    }

    /// Allocation libre précise, verrouillée pour l'assignation.
    pub async fn lock_free(
        conn: &mut PgConnection,
        node_id: Uuid,
        id: Uuid,
    ) -> Result<Option<(String, i32)>, sqlx::Error> {
        let row = sqlx::query!(
            "SELECT ip, port FROM allocations
             WHERE node_id = $1 AND id = $2 AND server_id IS NULL
             FOR UPDATE",
            node_id,
            id,
        )
        .fetch_optional(conn)
        .await?;
        Ok(row.map(|r| (r.ip, r.port)))
    }

    /// Première allocation libre du nœud, verrouillée (sans attendre celles déjà prises).
    pub async fn lock_first_free(
        conn: &mut PgConnection,
        node_id: Uuid,
        exclude: &[Uuid],
    ) -> Result<Option<(Uuid, String, i32)>, sqlx::Error> {
        let row = sqlx::query!(
            "SELECT id, ip, port FROM allocations
             WHERE node_id = $1 AND server_id IS NULL AND NOT (id = ANY($2))
             ORDER BY ip, port
             LIMIT 1
             FOR UPDATE SKIP LOCKED",
            node_id,
            exclude,
        )
        .fetch_optional(conn)
        .await?;
        Ok(row.map(|r| (r.id, r.ip, r.port)))
    }

    pub async fn assign(
        conn: &mut PgConnection,
        id: Uuid,
        server_id: Uuid,
        port_name: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE allocations SET server_id = $2, port_name = $3 WHERE id = $1",
            id,
            server_id,
            port_name,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Ports libres d'un nœud.
    pub async fn count_free(conn: &mut PgConnection, node_id: Uuid) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar!(
            r#"SELECT count(*) AS "count!" FROM allocations WHERE node_id = $1 AND server_id IS NULL"#,
            node_id,
        )
        .fetch_one(conn)
        .await
    }

    /// Rend libre l'allocation d'un port que le serveur n'utilise plus.
    pub async fn release(
        conn: &mut PgConnection,
        server_id: Uuid,
        port_name: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE allocations SET server_id = NULL, port_name = NULL
             WHERE server_id = $1 AND port_name = $2",
            server_id,
            port_name,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Allocations de plusieurs serveurs en une requête, par serveur : (port du prefab, adresse,
    /// port de l'hôte). Un serveur sans port est absent.
    pub async fn of_servers(
        conn: &mut PgConnection,
        server_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, Vec<(String, String, i32)>>, sqlx::Error> {
        let rows = sqlx::query!(
            r#"SELECT server_id AS "server_id!", port_name AS "port_name!", ip, port
               FROM allocations
               WHERE server_id = ANY($1) ORDER BY server_id, port_name"#,
            server_ids,
        )
        .fetch_all(conn)
        .await?;
        let mut by_server: HashMap<Uuid, Vec<(String, String, i32)>> = HashMap::new();
        for row in rows {
            by_server
                .entry(row.server_id)
                .or_default()
                .push((row.port_name, row.ip, row.port));
        }
        Ok(by_server)
    }

    /// Allocations d'un serveur : port du prefab → (adresse, port de l'hôte).
    pub async fn of_server(
        conn: &mut PgConnection,
        server_id: Uuid,
    ) -> Result<Vec<(String, String, i32)>, sqlx::Error> {
        let rows = sqlx::query!(
            r#"SELECT port_name AS "port_name!", ip, port FROM allocations
               WHERE server_id = $1 ORDER BY port_name"#,
            server_id,
        )
        .fetch_all(conn)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| (r.port_name, r.ip, r.port))
            .collect())
    }
}
