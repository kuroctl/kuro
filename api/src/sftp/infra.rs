//! Accès aux tables SFTP (ADR-0047) : clés des comptes, identifiants des noms d'utilisateur,
//! point d'accès des nœuds, accusés des événements remontés.

use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct KeyRecord {
    pub id: Uuid,
    pub name: String,
    pub algorithm: String,
    pub fingerprint: String,
    pub created_at: OffsetDateTime,
    pub last_used_at: Option<OffsetDateTime>,
}

/// Compte désigné par un nom d'utilisateur SFTP.
#[derive(Debug, Clone)]
pub struct AccountRecord {
    pub id: Uuid,
    pub disabled: bool,
}

/// Serveur désigné par un nom d'utilisateur SFTP.
#[derive(Debug, Clone)]
pub struct ServerRecord {
    pub id: Uuid,
    pub node_id: Uuid,
    pub suspended: bool,
}

/// Coordonnées de connexion d'un serveur, pour le panel.
#[derive(Debug, Clone)]
pub struct EndpointRecord {
    pub server_handle: String,
    pub node_name: String,
    /// Nom DNS du nœud publié par Kuro (ADR-0026), sinon son adresse publique saisie.
    pub host: Option<String>,
    /// Adresse vue sur le lien du nœud, dernier repli.
    pub link_address: Option<String>,
    pub port: Option<i32>,
    pub host_key: Option<String>,
}

#[derive(Debug)]
pub struct SftpRepository;

impl SftpRepository {
    pub async fn keys(
        conn: &mut PgConnection,
        admin_id: Uuid,
    ) -> Result<Vec<KeyRecord>, sqlx::Error> {
        sqlx::query_as!(
            KeyRecord,
            "SELECT id, name, algorithm, fingerprint, created_at, last_used_at
             FROM sftp_keys WHERE admin_id = $1 ORDER BY created_at",
            admin_id,
        )
        .fetch_all(conn)
        .await
    }

    /// `false` : le compte a déjà cette clé.
    pub async fn insert_key(
        conn: &mut PgConnection,
        id: Uuid,
        admin_id: Uuid,
        name: &str,
        key: &super::domain::PublicKey,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query!(
            "INSERT INTO sftp_keys (id, admin_id, name, algorithm, public_key, fingerprint)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (admin_id, fingerprint) DO NOTHING",
            id,
            admin_id,
            name,
            key.algorithm,
            key.blob,
            key.fingerprint,
        )
        .execute(conn)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Clé supprimée (empreinte), `None` si le compte n'a pas cette clé.
    pub async fn delete_key(
        conn: &mut PgConnection,
        admin_id: Uuid,
        id: Uuid,
    ) -> Result<Option<String>, sqlx::Error> {
        sqlx::query_scalar!(
            "DELETE FROM sftp_keys WHERE id = $1 AND admin_id = $2 RETURNING fingerprint",
            id,
            admin_id,
        )
        .fetch_optional(conn)
        .await
    }

    /// Clé du compte dont le blob est exactement celui présenté.
    pub async fn key_by_blob(
        conn: &mut PgConnection,
        admin_id: Uuid,
        fingerprint: &str,
        blob: &str,
    ) -> Result<Option<Uuid>, sqlx::Error> {
        sqlx::query_scalar!(
            "SELECT id FROM sftp_keys
             WHERE admin_id = $1 AND fingerprint = $2 AND public_key = $3",
            admin_id,
            fingerprint,
            blob,
        )
        .fetch_optional(conn)
        .await
    }

    pub async fn key_exists(
        conn: &mut PgConnection,
        admin_id: Uuid,
        id: Uuid,
    ) -> Result<bool, sqlx::Error> {
        Ok(sqlx::query_scalar!(
            "SELECT EXISTS (SELECT 1 FROM sftp_keys WHERE id = $1 AND admin_id = $2)",
            id,
            admin_id,
        )
        .fetch_one(conn)
        .await?
        .unwrap_or(false))
    }

    pub async fn touch_key(conn: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE sftp_keys SET last_used_at = now() WHERE id = $1",
            id
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn account_by_handle(
        conn: &mut PgConnection,
        handle: &str,
    ) -> Result<Option<AccountRecord>, sqlx::Error> {
        sqlx::query_as!(
            AccountRecord,
            r#"SELECT id, disabled_at IS NOT NULL AS "disabled!" FROM admin_users
               WHERE sftp_handle = $1"#,
            handle,
        )
        .fetch_optional(conn)
        .await
    }

    pub async fn account(
        conn: &mut PgConnection,
        id: Uuid,
    ) -> Result<Option<AccountRecord>, sqlx::Error> {
        sqlx::query_as!(
            AccountRecord,
            r#"SELECT id, disabled_at IS NOT NULL AS "disabled!" FROM admin_users WHERE id = $1"#,
            id,
        )
        .fetch_optional(conn)
        .await
    }

    pub async fn account_handle(
        conn: &mut PgConnection,
        id: Uuid,
    ) -> Result<Option<String>, sqlx::Error> {
        sqlx::query_scalar!("SELECT sftp_handle FROM admin_users WHERE id = $1", id)
            .fetch_optional(conn)
            .await
    }

    pub async fn server_by_handle(
        conn: &mut PgConnection,
        handle: &str,
    ) -> Result<Option<ServerRecord>, sqlx::Error> {
        sqlx::query_as!(
            ServerRecord,
            r#"SELECT id, node_id, suspended_at IS NOT NULL AS "suspended!" FROM servers
               WHERE sftp_handle = $1"#,
            handle,
        )
        .fetch_optional(conn)
        .await
    }

    pub async fn server(
        conn: &mut PgConnection,
        id: Uuid,
    ) -> Result<Option<ServerRecord>, sqlx::Error> {
        sqlx::query_as!(
            ServerRecord,
            r#"SELECT id, node_id, suspended_at IS NOT NULL AS "suspended!" FROM servers
               WHERE id = $1"#,
            id,
        )
        .fetch_optional(conn)
        .await
    }

    pub async fn endpoint(
        conn: &mut PgConnection,
        server_id: Uuid,
    ) -> Result<Option<EndpointRecord>, sqlx::Error> {
        sqlx::query_as!(
            EndpointRecord,
            r#"SELECT s.sftp_handle AS server_handle, n.name AS node_name,
                      COALESCE(
                          (SELECT r.name FROM dns_records r
                           WHERE r.owner_kind = 'node' AND r.owner_id = n.id
                             AND r.kind IN ('A', 'AAAA')
                           ORDER BY r.kind LIMIT 1),
                          n.public_ipv4, n.public_ipv6) AS host,
                      n.link_address,
                      n.sftp_port AS port, n.sftp_host_key AS host_key
               FROM servers s JOIN nodes n ON n.id = s.node_id
               WHERE s.id = $1"#,
            server_id,
        )
        .fetch_optional(conn)
        .await
    }

    /// `false` : événement déjà enregistré (lot renvoyé).
    pub async fn receive(conn: &mut PgConnection, event_id: Uuid) -> Result<bool, sqlx::Error> {
        let result = sqlx::query!(
            "INSERT INTO sftp_audit_receipts (event_id) VALUES ($1) ON CONFLICT DO NOTHING",
            event_id,
        )
        .execute(conn)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn purge_receipts(conn: &mut PgConnection) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!(
            "DELETE FROM sftp_audit_receipts WHERE received_at < now() - interval '7 days'"
        )
        .execute(conn)
        .await?;
        Ok(result.rows_affected())
    }
}
