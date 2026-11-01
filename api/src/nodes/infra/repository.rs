use sqlx::{PgConnection, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::nodes::domain::{
    CertFingerprint, InstanceId, Node, NodeHost, NodeId, NodeName, NodeStatus, Placement,
};

/// Vue complète d'un nœud pour l'administration (région jointe).
#[derive(Debug, Clone)]
pub struct NodeRecord {
    pub id: Uuid,
    pub name: String,
    pub display_name: Option<String>,
    pub status: String,
    pub region_id: Option<Uuid>,
    pub region_slug: Option<String>,
    pub region_name: Option<String>,
    pub region_latitude: Option<f64>,
    pub region_longitude: Option<f64>,
    pub location: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub daemon_version: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub cpu_cores: Option<i32>,
    pub memory_bytes: Option<i64>,
    pub last_heartbeat_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    /// Accepte les essais du Studio.
    pub trials: bool,
    /// Adresses publiques, publiées sous le nom du nœud (ADR-0026).
    pub public_ipv4: Option<String>,
    pub public_ipv6: Option<String>,
}

/// Nœud verrouillé pour une décision sur son cycle de vie.
#[derive(Debug, Clone)]
pub struct LockedNode {
    pub name: String,
    pub status: NodeStatus,
    /// Absent tant que le nœud n'est pas enrôlé.
    pub cert_fingerprint: Option<String>,
}

/// Filtre de liste : tous, une région, ou les nœuds sans région.
#[derive(Debug, Clone, Copy, Default)]
pub enum RegionFilter {
    #[default]
    All,
    Region(Uuid),
    Unassigned,
}

/// Modification partielle : `None` laisse le champ inchangé, `Some(None)` l'efface.
#[derive(Debug, Clone, Default)]
pub struct PlacementPatch {
    pub region_id: Option<Option<Uuid>>,
    pub display_name: Option<Option<String>>,
    pub location: Option<Option<String>>,
    pub coordinates: Option<Option<(f64, f64)>>,
}

/// Persistance des nœuds (table `nodes`).
///
/// Les fonctions associées prenant une connexion s'utilisent dans une transaction
/// ouverte par un cas d'usage qui touche plusieurs tables.
#[derive(Debug, Clone)]
pub struct NodeRepository {
    pool: PgPool,
}

impl NodeRepository {
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn insert_pending(
        conn: &mut PgConnection,
        id: NodeId,
        name: &NodeName,
        placement: &Placement,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO nodes (id, name, status, region_id, display_name, location, latitude, longitude)
             VALUES ($1, $2, 'pending', $3, $4, $5, $6, $7)",
            id.as_uuid(),
            name.as_str(),
            placement.region_id,
            placement.display_name,
            placement.location,
            placement.coordinates.map(|c| c.latitude),
            placement.coordinates.map(|c| c.longitude),
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn list(
        conn: &mut PgConnection,
        filter: RegionFilter,
    ) -> Result<Vec<NodeRecord>, sqlx::Error> {
        let (region, unassigned) = match filter {
            RegionFilter::All => (None, false),
            RegionFilter::Region(id) => (Some(id), false),
            RegionFilter::Unassigned => (None, true),
        };
        sqlx::query_as!(
            NodeRecord,
            r#"SELECT n.id, n.name, n.display_name, n.status, n.region_id,
                      r.slug AS "region_slug?", r.name AS "region_name?",
                      r.latitude AS "region_latitude?", r.longitude AS "region_longitude?",
                      n.location, n.latitude, n.longitude, n.daemon_version, n.os, n.arch,
                      n.cpu_cores, n.memory_bytes, n.last_heartbeat_at, n.created_at, n.trials, n.public_ipv4, n.public_ipv6
               FROM nodes n LEFT JOIN regions r ON r.id = n.region_id
               WHERE ($1::uuid IS NULL OR n.region_id = $1)
                 AND (NOT $2 OR n.region_id IS NULL)
               ORDER BY n.name"#,
            region,
            unassigned,
        )
        .fetch_all(conn)
        .await
    }

    /// Accepte ou non les essais du Studio.
    pub async fn set_trials(
        conn: &mut PgConnection,
        id: Uuid,
        trials: bool,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!("UPDATE nodes SET trials = $2 WHERE id = $1", id, trials)
            .execute(conn)
            .await?;
        Ok(())
    }

    pub async fn get(conn: &mut PgConnection, id: Uuid) -> Result<Option<NodeRecord>, sqlx::Error> {
        sqlx::query_as!(
            NodeRecord,
            r#"SELECT n.id, n.name, n.display_name, n.status, n.region_id,
                      r.slug AS "region_slug?", r.name AS "region_name?",
                      r.latitude AS "region_latitude?", r.longitude AS "region_longitude?",
                      n.location, n.latitude, n.longitude, n.daemon_version, n.os, n.arch,
                      n.cpu_cores, n.memory_bytes, n.last_heartbeat_at, n.created_at, n.trials, n.public_ipv4, n.public_ipv6
               FROM nodes n LEFT JOIN regions r ON r.id = n.region_id
               WHERE n.id = $1"#,
            id,
        )
        .fetch_optional(conn)
        .await
    }

    pub async fn id_by_name(
        conn: &mut PgConnection,
        name: &str,
    ) -> Result<Option<Uuid>, sqlx::Error> {
        sqlx::query_scalar!("SELECT id FROM nodes WHERE name = $1", name)
            .fetch_optional(conn)
            .await
    }

    /// Applique une modification partielle de l'emplacement et du libellé.
    pub async fn update_placement(
        conn: &mut PgConnection,
        id: Uuid,
        patch: &PlacementPatch,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE nodes SET
                region_id    = CASE WHEN $2 THEN $3 ELSE region_id END,
                display_name = CASE WHEN $4 THEN $5 ELSE display_name END,
                location     = CASE WHEN $6 THEN $7 ELSE location END,
                latitude     = CASE WHEN $8 THEN $9 ELSE latitude END,
                longitude    = CASE WHEN $8 THEN $10 ELSE longitude END,
                updated_at   = now()
             WHERE id = $1",
            id,
            patch.region_id.is_some(),
            patch.region_id.flatten(),
            patch.display_name.is_some(),
            patch.display_name.clone().flatten(),
            patch.location.is_some(),
            patch.location.clone().flatten(),
            patch.coordinates.is_some(),
            patch.coordinates.flatten().map(|(latitude, _)| latitude),
            patch.coordinates.flatten().map(|(_, longitude)| longitude),
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Nœud `pending`, verrouillé jusqu'à la fin de la transaction.
    pub async fn lock_pending(
        conn: &mut PgConnection,
        id: NodeId,
    ) -> Result<Option<Node>, sqlx::Error> {
        let row = sqlx::query!(
            "SELECT id, name FROM nodes WHERE id = $1 AND status = 'pending' FOR UPDATE",
            id.as_uuid(),
        )
        .fetch_optional(conn)
        .await?;
        Ok(row.map(|row| Node {
            id: NodeId::from(row.id),
            name: row.name,
            status: NodeStatus::Pending,
        }))
    }

    /// Nom, statut et certificat du nœud, verrouillés jusqu'à la fin de la transaction.
    pub async fn lock(
        conn: &mut PgConnection,
        id: Uuid,
    ) -> Result<Option<LockedNode>, sqlx::Error> {
        let row = sqlx::query!(
            "SELECT name, status, cert_fingerprint FROM nodes WHERE id = $1 FOR UPDATE",
            id
        )
        .fetch_optional(conn)
        .await?;
        row.map(|row| {
            Ok(LockedNode {
                name: row.name,
                status: row
                    .status
                    .parse()
                    .map_err(|error| sqlx::Error::Decode(Box::new(error)))?,
                cert_fingerprint: row.cert_fingerprint,
            })
        })
        .transpose()
    }

    /// Serveurs encore assignés au nœud : ils bloquent sa suppression.
    pub async fn count_servers(conn: &mut PgConnection, id: Uuid) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar!(
            r#"SELECT count(*) AS "count!" FROM servers WHERE node_id = $1"#,
            id
        )
        .fetch_one(conn)
        .await
    }

    /// Le certificat d'un nœud supprimé reste refusé, comme révoqué (ADR-0046).
    pub async fn remember_revoked_certificate(
        conn: &mut PgConnection,
        fingerprint: &str,
        node_name: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO revoked_certificates (fingerprint, node_name) VALUES ($1, $2)
             ON CONFLICT (fingerprint) DO NOTHING",
            fingerprint,
            node_name,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Certificat d'un nœud supprimé, présenté sur le lien daemon.
    pub async fn certificate_revoked(
        &self,
        fingerprint: &CertFingerprint,
    ) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar!(
            r#"SELECT EXISTS (SELECT 1 FROM revoked_certificates WHERE fingerprint = $1) AS "revoked!""#,
            fingerprint.as_str(),
        )
        .fetch_one(&self.pool)
        .await
    }

    /// Statut du nœud, verrouillé jusqu'à la fin de la transaction.
    pub async fn lock_status(
        conn: &mut PgConnection,
        id: Uuid,
    ) -> Result<Option<NodeStatus>, sqlx::Error> {
        let status = sqlx::query_scalar!("SELECT status FROM nodes WHERE id = $1 FOR UPDATE", id)
            .fetch_optional(conn)
            .await?;
        status
            .map(|status| {
                status
                    .parse()
                    .map_err(|error| sqlx::Error::Decode(Box::new(error)))
            })
            .transpose()
    }

    /// L'empreinte est conservée : le nœud reste reconnu, pour être refusé.
    pub async fn revoke(conn: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE nodes SET status = 'revoked', updated_at = now() WHERE id = $1",
            id
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Repart de zéro pour un réenrôlement : `pending`, plus de certificat rattaché.
    pub async fn reset_for_enrollment(
        conn: &mut PgConnection,
        id: Uuid,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE nodes SET status = 'pending', cert_fingerprint = NULL, updated_at = now()
             WHERE id = $1",
            id
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn delete(conn: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query!("DELETE FROM nodes WHERE id = $1", id)
            .execute(conn)
            .await?;
        Ok(())
    }

    /// Enrôlement terminé : le certificat est rattaché, le nœud attend son premier `hello`.
    pub async fn complete_enrollment(
        conn: &mut PgConnection,
        id: NodeId,
        fingerprint: &CertFingerprint,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE nodes SET status = 'offline', cert_fingerprint = $2, updated_at = now() WHERE id = $1",
            id.as_uuid(),
            fingerprint.as_str(),
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn find_by_fingerprint(
        &self,
        fingerprint: &CertFingerprint,
    ) -> Result<Option<Node>, sqlx::Error> {
        let row = sqlx::query!(
            "SELECT id, name, status FROM nodes WHERE cert_fingerprint = $1",
            fingerprint.as_str(),
        )
        .fetch_optional(&self.pool)
        .await?;

        row.map(|row| {
            Ok(Node {
                id: NodeId::from(row.id),
                name: row.name,
                status: row
                    .status
                    .parse()
                    .map_err(|error| sqlx::Error::Decode(Box::new(error)))?,
            })
        })
        .transpose()
    }

    /// Retourne `false` si le nœud n'est plus éligible (révoqué ou supprimé entre-temps).
    pub async fn mark_online(
        &self,
        id: NodeId,
        host: &NodeHost,
        instance: InstanceId,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query!(
            "UPDATE nodes
             SET status = 'online', daemon_version = $2, os = $3, arch = $4,
                 cpu_cores = $5, memory_bytes = $6, instance_id = $7,
                 sftp_port = $8, sftp_host_key = $9, link_address = $10,
                 last_heartbeat_at = now(), updated_at = now()
             WHERE id = $1 AND status IN ('online', 'offline')",
            id.as_uuid(),
            host.daemon_version,
            host.os,
            host.arch,
            i32::try_from(host.cpu_cores).unwrap_or(i32::MAX),
            i64::try_from(host.memory_bytes).unwrap_or(i64::MAX),
            instance.as_uuid(),
            host.sftp.as_ref().map(|(port, _)| i32::from(*port)),
            host.sftp.as_ref().map(|(_, key)| key.as_str()),
            host.link_address.as_deref(),
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Statut du nœud, `None` s'il a été supprimé. Lecture seule : les heartbeats s'écrivent
    /// par lots ([`Self::touch_heartbeats`]), pas un `UPDATE` par nœud toutes les 15 s.
    pub async fn status_of(&self, id: NodeId) -> Result<Option<String>, sqlx::Error> {
        sqlx::query_scalar!("SELECT status FROM nodes WHERE id = $1", id.as_uuid())
            .fetch_optional(&self.pool)
            .await
    }

    /// Date du dernier heartbeat des nœuds `online` donnés, en une requête.
    pub async fn touch_heartbeats(&self, ids: &[Uuid]) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!(
            "UPDATE nodes SET last_heartbeat_at = now()
             WHERE id = ANY($1) AND status = 'online'",
            ids,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn mark_offline(&self, id: NodeId) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE nodes SET status = 'offline', instance_id = NULL, updated_at = now()
             WHERE id = $1 AND status = 'online'",
            id.as_uuid(),
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Instance qui tient la session du nœud, s'il est `online` quelque part (ADR-0037).
    pub async fn held_by(&self, id: NodeId) -> Result<Option<InstanceId>, sqlx::Error> {
        Ok(sqlx::query_scalar!(
            "SELECT instance_id FROM nodes WHERE id = $1 AND status = 'online'",
            id.as_uuid()
        )
        .fetch_optional(&self.pool)
        .await?
        .flatten()
        .map(InstanceId::from))
    }
}
