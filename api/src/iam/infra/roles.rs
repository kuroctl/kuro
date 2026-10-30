use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct RoleRecord {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub color: String,
    pub is_system: bool,
    pub permissions: Vec<String>,
    pub members: i64,
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Clone)]
pub struct RoleRef {
    pub user_id: Uuid,
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug)]
pub struct RoleRepository;

impl RoleRepository {
    pub async fn insert(
        conn: &mut PgConnection,
        id: Uuid,
        name: &str,
        description: &str,
        color: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO roles (id, name, description, color) VALUES ($1, $2, $3, $4)",
            id,
            name,
            description,
            color,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn update(
        conn: &mut PgConnection,
        id: Uuid,
        name: Option<&str>,
        description: Option<&str>,
        color: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE roles SET name = COALESCE($2, name), description = COALESCE($3, description),
                              color = COALESCE($4, color),
                              updated_at = now()
             WHERE id = $1",
            id,
            name,
            description,
            color,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn set_permissions(
        conn: &mut PgConnection,
        id: Uuid,
        permissions: &[&str],
    ) -> Result<(), sqlx::Error> {
        let permissions: Vec<String> = permissions.iter().map(|key| (*key).to_owned()).collect();
        sqlx::query!("DELETE FROM role_permissions WHERE role_id = $1", id)
            .execute(&mut *conn)
            .await?;
        sqlx::query!(
            "INSERT INTO role_permissions (role_id, permission) SELECT $1, unnest($2::text[])",
            id,
            &permissions,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn delete(conn: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query!("DELETE FROM roles WHERE id = $1", id)
            .execute(conn)
            .await?;
        Ok(())
    }

    pub async fn get(conn: &mut PgConnection, id: Uuid) -> Result<Option<RoleRecord>, sqlx::Error> {
        sqlx::query_as!(
            RoleRecord,
            r#"SELECT r.id, r.name, r.description, r.color, r.is_system, r.created_at,
                      COALESCE(array_agg(DISTINCT rp.permission) FILTER (WHERE rp.permission IS NOT NULL), '{}') AS "permissions!",
                      (SELECT count(*) FROM admin_user_roles ur WHERE ur.role_id = r.id) AS "members!"
               FROM roles r LEFT JOIN role_permissions rp ON rp.role_id = r.id
               WHERE r.id = $1
               GROUP BY r.id"#,
            id,
        )
        .fetch_optional(conn)
        .await
    }

    pub async fn list(conn: &mut PgConnection) -> Result<Vec<RoleRecord>, sqlx::Error> {
        sqlx::query_as!(
            RoleRecord,
            r#"SELECT r.id, r.name, r.description, r.color, r.is_system, r.created_at,
                      COALESCE(array_agg(DISTINCT rp.permission) FILTER (WHERE rp.permission IS NOT NULL), '{}') AS "permissions!",
                      (SELECT count(*) FROM admin_user_roles ur WHERE ur.role_id = r.id) AS "members!"
               FROM roles r LEFT JOIN role_permissions rp ON rp.role_id = r.id
               GROUP BY r.id
               ORDER BY r.is_system DESC, r.name"#,
        )
        .fetch_all(conn)
        .await
    }

    /// Clés de permission accumulées par un ensemble de rôles.
    pub async fn permission_keys(
        conn: &mut PgConnection,
        role_ids: &[Uuid],
    ) -> Result<Vec<String>, sqlx::Error> {
        sqlx::query_scalar!(
            "SELECT DISTINCT permission FROM role_permissions WHERE role_id = ANY($1)",
            role_ids,
        )
        .fetch_all(conn)
        .await
    }

    /// Nombre de rôles existants parmi `role_ids` (vérification d'existence).
    pub async fn count_existing(
        conn: &mut PgConnection,
        role_ids: &[Uuid],
    ) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar!(
            r#"SELECT count(*) AS "count!" FROM roles WHERE id = ANY($1)"#,
            role_ids,
        )
        .fetch_one(conn)
        .await
    }

    /// Rôles des comptes donnés, pour l'affichage.
    pub async fn of_users(
        conn: &mut PgConnection,
        user_ids: &[Uuid],
    ) -> Result<Vec<RoleRef>, sqlx::Error> {
        sqlx::query_as!(
            RoleRef,
            "SELECT ur.user_id, r.id, r.name FROM admin_user_roles ur
             JOIN roles r ON r.id = ur.role_id
             WHERE ur.user_id = ANY($1)
             ORDER BY r.name",
            user_ids,
        )
        .fetch_all(conn)
        .await
    }
}
