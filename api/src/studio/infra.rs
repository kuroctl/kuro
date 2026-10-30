//! Stockage des brouillons du Studio (`prefab_drafts`, `prefab_draft_assets`).

use serde_json::Value;
use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

/// Brouillon stocké : fichiers texte et types d'images présents.
#[derive(Debug, Clone)]
pub struct DraftRecord {
    pub id: Uuid,
    /// Chemin relatif → contenu texte.
    pub files: Value,
    pub prefab_id: Option<String>,
    pub prefab_version: Option<String>,
    pub base_id: Option<String>,
    pub base_version: Option<String>,
    pub assets: Vec<String>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// Brouillon à créer.
#[derive(Debug)]
pub struct NewDraft<'a> {
    pub id: Uuid,
    pub files: &'a Value,
    pub prefab_id: Option<&'a str>,
    pub prefab_version: Option<&'a str>,
    pub base_id: Option<&'a str>,
    pub base_version: Option<&'a str>,
    pub author: Option<Uuid>,
}

#[derive(Debug)]
pub struct DraftRepository;

impl DraftRepository {
    pub async fn list(conn: &mut PgConnection) -> Result<Vec<DraftRecord>, sqlx::Error> {
        let rows = sqlx::query!(
            r#"SELECT d.id, d.files, d.prefab_id, d.prefab_version, d.base_id, d.base_version,
                      d.created_at, d.updated_at,
                      COALESCE(array_agg(a.kind ORDER BY a.kind) FILTER (WHERE a.kind IS NOT NULL),
                               '{}') AS "assets!: Vec<String>"
               FROM prefab_drafts d
               LEFT JOIN prefab_draft_assets a ON a.draft_id = d.id
               GROUP BY d.id
               ORDER BY d.updated_at DESC"#
        )
        .fetch_all(conn)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| DraftRecord {
                id: row.id,
                files: row.files,
                prefab_id: row.prefab_id,
                prefab_version: row.prefab_version,
                base_id: row.base_id,
                base_version: row.base_version,
                assets: row.assets,
                created_at: row.created_at,
                updated_at: row.updated_at,
            })
            .collect())
    }

    pub async fn get(
        conn: &mut PgConnection,
        id: Uuid,
    ) -> Result<Option<DraftRecord>, sqlx::Error> {
        let row = sqlx::query!(
            r#"SELECT d.id, d.files, d.prefab_id, d.prefab_version, d.base_id, d.base_version,
                      d.created_at, d.updated_at,
                      COALESCE(array_agg(a.kind ORDER BY a.kind) FILTER (WHERE a.kind IS NOT NULL),
                               '{}') AS "assets!: Vec<String>"
               FROM prefab_drafts d
               LEFT JOIN prefab_draft_assets a ON a.draft_id = d.id
               WHERE d.id = $1
               GROUP BY d.id"#,
            id,
        )
        .fetch_optional(conn)
        .await?;
        Ok(row.map(|row| DraftRecord {
            id: row.id,
            files: row.files,
            prefab_id: row.prefab_id,
            prefab_version: row.prefab_version,
            base_id: row.base_id,
            base_version: row.base_version,
            assets: row.assets,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }))
    }

    /// Verrouille le brouillon jusqu'à la fin de la transaction ; faux s'il n'existe pas.
    pub async fn lock(conn: &mut PgConnection, id: Uuid) -> Result<bool, sqlx::Error> {
        let row = sqlx::query!("SELECT id FROM prefab_drafts WHERE id = $1 FOR UPDATE", id)
            .fetch_optional(conn)
            .await?;
        Ok(row.is_some())
    }

    pub async fn insert(conn: &mut PgConnection, draft: &NewDraft<'_>) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO prefab_drafts
                 (id, files, prefab_id, prefab_version, base_id, base_version, created_by, updated_by)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $7)",
            draft.id,
            draft.files,
            draft.prefab_id,
            draft.prefab_version,
            draft.base_id,
            draft.base_version,
            draft.author,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Remplace les fichiers ; faux si le brouillon n'existe pas.
    pub async fn update_files(
        conn: &mut PgConnection,
        id: Uuid,
        files: &Value,
        prefab_id: Option<&str>,
        prefab_version: Option<&str>,
        author: Option<Uuid>,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query!(
            "UPDATE prefab_drafts
             SET files = $2, prefab_id = $3, prefab_version = $4, updated_by = $5, updated_at = now()
             WHERE id = $1",
            id,
            files,
            prefab_id,
            prefab_version,
            author,
        )
        .execute(conn)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Marque le brouillon modifié (une image a changé).
    pub async fn touch(
        conn: &mut PgConnection,
        id: Uuid,
        author: Option<Uuid>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE prefab_drafts SET updated_by = $2, updated_at = now() WHERE id = $1",
            id,
            author,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn delete(conn: &mut PgConnection, id: Uuid) -> Result<bool, sqlx::Error> {
        let result = sqlx::query!("DELETE FROM prefab_drafts WHERE id = $1", id)
            .execute(conn)
            .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn put_asset(
        conn: &mut PgConnection,
        id: Uuid,
        kind: &str,
        content_type: &str,
        data: &[u8],
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO prefab_draft_assets (draft_id, kind, content_type, data)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (draft_id, kind) DO UPDATE
             SET content_type = EXCLUDED.content_type, data = EXCLUDED.data",
            id,
            kind,
            content_type,
            data,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn delete_asset(
        conn: &mut PgConnection,
        id: Uuid,
        kind: &str,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query!(
            "DELETE FROM prefab_draft_assets WHERE draft_id = $1 AND kind = $2",
            id,
            kind,
        )
        .execute(conn)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Images du brouillon : (type, type de contenu, octets).
    pub async fn assets(
        conn: &mut PgConnection,
        id: Uuid,
    ) -> Result<Vec<(String, String, Vec<u8>)>, sqlx::Error> {
        let rows = sqlx::query!(
            "SELECT kind, content_type, data FROM prefab_draft_assets
             WHERE draft_id = $1 ORDER BY kind",
            id,
        )
        .fetch_all(conn)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| (row.kind, row.content_type, row.data))
            .collect())
    }
}
