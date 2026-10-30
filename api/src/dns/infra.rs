//! Persistance du DNS : zone gérée, enregistrements créés par Kuro, et ce qu'il faut lire des
//! nœuds et des serveurs pour calculer les enregistrements voulus.

use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ZoneRecord {
    pub id: Uuid,
    pub provider: String,
    pub base_domain: String,
    pub nodes_label: String,
    /// Domaine des sites web (ADR-0038) ; absent : aucun prefab web n'est déployable.
    pub sites_domain: Option<String>,
    pub provider_zone: String,
    /// Jeton chiffré.
    pub token: Vec<u8>,
    pub ttl: i32,
    pub enabled: bool,
    pub updated_at: OffsetDateTime,
}

pub struct ZoneFields<'a> {
    pub provider: &'a str,
    pub base_domain: &'a str,
    pub nodes_label: &'a str,
    pub sites_domain: Option<&'a str>,
    pub provider_zone: &'a str,
    pub token: &'a [u8],
    pub ttl: i32,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct RecordRow {
    pub id: Uuid,
    pub owner_kind: String,
    pub owner_id: Uuid,
    pub kind: String,
    pub name: String,
    pub content: String,
    pub provider_id: Option<String>,
    pub synced_at: Option<OffsetDateTime>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NodeAddress {
    pub id: Uuid,
    pub name: String,
    pub public_ipv4: Option<String>,
    pub public_ipv6: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ServerName {
    pub id: Uuid,
    pub name: String,
    pub node_name: String,
    pub prefab_id: String,
    pub prefab_version: String,
    pub dns_label: Option<String>,
    /// Identifiant du site (ADR-0038), pour les serveurs dont le prefab a un port web.
    pub web_id: Option<String>,
}

pub struct DnsRepository;

impl DnsRepository {
    /// La zone gérée (une seule pour l'instant).
    pub async fn zone(conn: &mut PgConnection) -> Result<Option<ZoneRecord>, sqlx::Error> {
        sqlx::query_as!(
            ZoneRecord,
            "SELECT id, provider, base_domain, nodes_label, sites_domain, provider_zone, token, ttl,
                    enabled, updated_at
             FROM dns_zones ORDER BY created_at LIMIT 1"
        )
        .fetch_optional(conn)
        .await
    }

    pub async fn insert_zone(
        conn: &mut PgConnection,
        id: Uuid,
        fields: &ZoneFields<'_>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO dns_zones (id, provider, base_domain, nodes_label, provider_zone, token, ttl,
                                    enabled, sites_domain)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            id,
            fields.provider,
            fields.base_domain,
            fields.nodes_label,
            fields.provider_zone,
            fields.token,
            fields.ttl,
            fields.enabled,
            fields.sites_domain,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn update_zone(
        conn: &mut PgConnection,
        id: Uuid,
        fields: &ZoneFields<'_>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE dns_zones SET provider = $2, base_domain = $3, nodes_label = $4,
                provider_zone = $5, token = $6, ttl = $7, enabled = $8, sites_domain = $9,
                updated_at = now()
             WHERE id = $1",
            id,
            fields.provider,
            fields.base_domain,
            fields.nodes_label,
            fields.provider_zone,
            fields.token,
            fields.ttl,
            fields.enabled,
            fields.sites_domain,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn delete_zone(conn: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query!("DELETE FROM dns_zones WHERE id = $1", id)
            .execute(conn)
            .await?;
        Ok(())
    }

    pub async fn records(
        conn: &mut PgConnection,
        zone: Uuid,
    ) -> Result<Vec<RecordRow>, sqlx::Error> {
        sqlx::query_as!(
            RecordRow,
            "SELECT id, owner_kind, owner_id, kind, name, content, provider_id, synced_at, error
             FROM dns_records WHERE zone_id = $1 ORDER BY name, kind",
            zone,
        )
        .fetch_all(conn)
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn insert_record(
        conn: &mut PgConnection,
        id: Uuid,
        zone: Uuid,
        owner_kind: &str,
        owner_id: Uuid,
        kind: &str,
        name: &str,
        content: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO dns_records (id, zone_id, owner_kind, owner_id, kind, name, content)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            id,
            zone,
            owner_kind,
            owner_id,
            kind,
            name,
            content,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Publié chez le fournisseur : son identifiant, contenu à jour, plus d'erreur.
    pub async fn mark_synced(
        conn: &mut PgConnection,
        id: Uuid,
        provider_id: &str,
        content: &str,
        owner_id: Uuid,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE dns_records SET provider_id = $2, content = $3, owner_id = $4, error = NULL,
                synced_at = now(), updated_at = now()
             WHERE id = $1",
            id,
            provider_id,
            content,
            owner_id,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn mark_error(
        conn: &mut PgConnection,
        id: Uuid,
        error: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE dns_records SET error = $2, updated_at = now() WHERE id = $1",
            id,
            error,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn delete_record(conn: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query!("DELETE FROM dns_records WHERE id = $1", id)
            .execute(conn)
            .await?;
        Ok(())
    }

    pub async fn nodes(conn: &mut PgConnection) -> Result<Vec<NodeAddress>, sqlx::Error> {
        sqlx::query_as!(
            NodeAddress,
            "SELECT id, name, public_ipv4, public_ipv6 FROM nodes WHERE status <> 'revoked'"
        )
        .fetch_all(conn)
        .await
    }

    pub async fn servers(conn: &mut PgConnection) -> Result<Vec<ServerName>, sqlx::Error> {
        sqlx::query_as!(
            ServerName,
            "SELECT s.id, s.name, n.name AS node_name, s.prefab_id, s.prefab_version, s.dns_label,
                    s.web_id
             FROM servers s JOIN nodes n ON n.id = s.node_id
             WHERE s.state <> 'deleting'"
        )
        .fetch_all(conn)
        .await
    }

    /// Donne son label à un serveur ; `false` s'il est déjà pris.
    pub async fn set_label(
        conn: &mut PgConnection,
        server: Uuid,
        label: Option<&str>,
    ) -> Result<bool, sqlx::Error> {
        let taken = match label {
            Some(label) => sqlx::query_scalar!(
                r#"SELECT EXISTS (SELECT 1 FROM servers WHERE dns_label = $1 AND id <> $2) AS "taken!""#,
                label,
                server,
            )
            .fetch_one(&mut *conn)
            .await?,
            None => false,
        };
        if taken {
            return Ok(false);
        }
        sqlx::query!(
            "UPDATE servers SET dns_label = $2, updated_at = now() WHERE id = $1",
            server,
            label,
        )
        .execute(conn)
        .await?;
        Ok(true)
    }

    pub async fn labels(conn: &mut PgConnection) -> Result<Vec<String>, sqlx::Error> {
        sqlx::query_scalar!(
            r#"SELECT dns_label AS "label!" FROM servers WHERE dns_label IS NOT NULL"#
        )
        .fetch_all(conn)
        .await
    }

    pub async fn set_node_addresses(
        conn: &mut PgConnection,
        node: Uuid,
        ipv4: Option<&str>,
        ipv6: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE nodes SET public_ipv4 = $2, public_ipv6 = $3 WHERE id = $1",
            node,
            ipv4,
            ipv6,
        )
        .execute(conn)
        .await?;
        Ok(())
    }
}
