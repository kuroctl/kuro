use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

/// Région avec le décompte de ses nœuds par état.
#[derive(Debug, Clone)]
pub struct RegionRecord {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
    pub nodes_total: i64,
    pub nodes_online: i64,
    pub nodes_offline: i64,
    pub nodes_pending: i64,
    pub nodes_revoked: i64,
    pub created_at: OffsetDateTime,
}

#[derive(Debug)]
pub struct RegionRepository;

impl RegionRepository {
    pub async fn insert(
        conn: &mut PgConnection,
        id: Uuid,
        slug: &str,
        name: &str,
        latitude: f64,
        longitude: f64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO regions (id, slug, name, latitude, longitude) VALUES ($1, $2, $3, $4, $5)",
            id,
            slug,
            name,
            latitude,
            longitude,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn update(
        conn: &mut PgConnection,
        id: Uuid,
        name: Option<&str>,
        coordinates: Option<(f64, f64)>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE regions SET name = COALESCE($2, name),
                                latitude = COALESCE($3, latitude),
                                longitude = COALESCE($4, longitude),
                                updated_at = now()
             WHERE id = $1",
            id,
            name,
            coordinates.map(|(latitude, _)| latitude),
            coordinates.map(|(_, longitude)| longitude),
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn delete(conn: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query!("DELETE FROM regions WHERE id = $1", id)
            .execute(conn)
            .await?;
        Ok(())
    }

    pub async fn list(conn: &mut PgConnection) -> Result<Vec<RegionRecord>, sqlx::Error> {
        sqlx::query_as!(
            RegionRecord,
            r#"SELECT r.id, r.slug, r.name, r.latitude, r.longitude, r.created_at,
                      count(n.id) AS "nodes_total!",
                      count(n.id) FILTER (WHERE n.status = 'online') AS "nodes_online!",
                      count(n.id) FILTER (WHERE n.status = 'offline') AS "nodes_offline!",
                      count(n.id) FILTER (WHERE n.status = 'pending') AS "nodes_pending!",
                      count(n.id) FILTER (WHERE n.status = 'revoked') AS "nodes_revoked!"
               FROM regions r LEFT JOIN nodes n ON n.region_id = r.id
               GROUP BY r.id
               ORDER BY r.name"#,
        )
        .fetch_all(conn)
        .await
    }

    pub async fn get(
        conn: &mut PgConnection,
        id: Uuid,
    ) -> Result<Option<RegionRecord>, sqlx::Error> {
        sqlx::query_as!(
            RegionRecord,
            r#"SELECT r.id, r.slug, r.name, r.latitude, r.longitude, r.created_at,
                      count(n.id) AS "nodes_total!",
                      count(n.id) FILTER (WHERE n.status = 'online') AS "nodes_online!",
                      count(n.id) FILTER (WHERE n.status = 'offline') AS "nodes_offline!",
                      count(n.id) FILTER (WHERE n.status = 'pending') AS "nodes_pending!",
                      count(n.id) FILTER (WHERE n.status = 'revoked') AS "nodes_revoked!"
               FROM regions r LEFT JOIN nodes n ON n.region_id = r.id
               WHERE r.id = $1
               GROUP BY r.id"#,
            id,
        )
        .fetch_optional(conn)
        .await
    }

    pub async fn id_by_slug(
        conn: &mut PgConnection,
        slug: &str,
    ) -> Result<Option<Uuid>, sqlx::Error> {
        sqlx::query_scalar!("SELECT id FROM regions WHERE slug = $1", slug)
            .fetch_optional(conn)
            .await
    }

    pub async fn exists(conn: &mut PgConnection, id: Uuid) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar!(
            r#"SELECT EXISTS (SELECT 1 FROM regions WHERE id = $1) AS "exists!""#,
            id
        )
        .fetch_one(conn)
        .await
    }
}
