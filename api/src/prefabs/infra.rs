use serde_json::Value;
use sqlx::PgConnection;
use time::OffsetDateTime;

/// Prefab stocké : manifest, traductions et gabarits analysés.
#[derive(Debug, Clone)]
pub struct PrefabRecord {
    pub id: String,
    pub version: String,
    /// JSON du manifest, dans l'ordre du fichier.
    pub manifest: String,
    pub locales: Value,
    pub templates: Value,
    pub imported_at: OffsetDateTime,
    /// `official` (dossier `prefabs/`), `uploaded` (archive importée depuis le panel), `store`
    /// (installé depuis un store, ADR-0042) ou `trial` (essai d'un brouillon).
    pub source: String,
    /// Store d'origine d'une version `store`.
    pub store: Option<String>,
}

/// Prefab à stocker.
#[derive(Debug)]
pub struct NewPrefab<'a> {
    pub id: &'a str,
    pub version: &'a str,
    pub manifest: &'a str,
    pub locales: &'a Value,
    pub templates: &'a Value,
    pub source: &'a str,
    pub imported_by: Option<uuid::Uuid>,
    /// Brouillon figé par une version d'essai.
    pub draft_id: Option<uuid::Uuid>,
    /// Store d'origine (source `store`).
    pub store: Option<&'a str>,
}

#[derive(Debug)]
pub struct PrefabRepository;

impl PrefabRepository {
    pub async fn get(
        conn: &mut PgConnection,
        id: &str,
        version: &str,
    ) -> Result<Option<PrefabRecord>, sqlx::Error> {
        sqlx::query_as!(
            PrefabRecord,
            "SELECT id, version, manifest, locales, templates, imported_at, source, store
             FROM prefabs WHERE id = $1 AND version = $2",
            id,
            version,
        )
        .fetch_optional(conn)
        .await
    }

    pub async fn list(conn: &mut PgConnection) -> Result<Vec<PrefabRecord>, sqlx::Error> {
        sqlx::query_as!(
            PrefabRecord,
            "SELECT id, version, manifest, locales, templates, imported_at, source, store
             FROM prefabs WHERE source <> 'trial' ORDER BY id, imported_at",
        )
        .fetch_all(conn)
        .await
    }

    /// Image d'un prefab : type et octets.
    pub async fn asset(
        conn: &mut PgConnection,
        id: &str,
        version: &str,
        kind: &str,
    ) -> Result<Option<(String, Vec<u8>)>, sqlx::Error> {
        let row = sqlx::query!(
            "SELECT content_type, data FROM prefab_assets
             WHERE prefab_id = $1 AND prefab_version = $2 AND kind = $3",
            id,
            version,
            kind,
        )
        .fetch_optional(conn)
        .await?;
        Ok(row.map(|row| (row.content_type, row.data)))
    }

    /// Images d'une version : (type d'image, octets).
    pub async fn assets_of(
        conn: &mut PgConnection,
        id: &str,
        version: &str,
    ) -> Result<Vec<(String, Vec<u8>)>, sqlx::Error> {
        let rows = sqlx::query!(
            "SELECT kind, data FROM prefab_assets
             WHERE prefab_id = $1 AND prefab_version = $2 ORDER BY kind",
            id,
            version,
        )
        .fetch_all(conn)
        .await?;
        Ok(rows.into_iter().map(|row| (row.kind, row.data)).collect())
    }

    /// Remplace les octets d'une image par sa version optimisée (même format).
    pub async fn replace_asset(
        conn: &mut PgConnection,
        id: &str,
        version: &str,
        kind: &str,
        data: &[u8],
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE prefab_assets SET data = $4
             WHERE prefab_id = $1 AND prefab_version = $2 AND kind = $3",
            id,
            version,
            kind,
            data,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Images présentes, par prefab : (id, version, type d'image).
    pub async fn asset_kinds(
        conn: &mut PgConnection,
    ) -> Result<Vec<(String, String, String)>, sqlx::Error> {
        let rows =
            sqlx::query!("SELECT prefab_id, prefab_version, kind FROM prefab_assets ORDER BY kind")
                .fetch_all(conn)
                .await?;
        Ok(rows
            .into_iter()
            .map(|row| (row.prefab_id, row.prefab_version, row.kind))
            .collect())
    }

    /// Ajoute une image si le prefab n'en a pas encore de ce type. Vrai si elle a été ajoutée.
    pub async fn insert_asset_if_absent(
        conn: &mut PgConnection,
        id: &str,
        version: &str,
        kind: &str,
        content_type: &str,
        data: &[u8],
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query!(
            "INSERT INTO prefab_assets (prefab_id, prefab_version, kind, content_type, data)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT DO NOTHING",
            id,
            version,
            kind,
            content_type,
            data,
        )
        .execute(conn)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn insert(
        conn: &mut PgConnection,
        prefab: &NewPrefab<'_>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO prefabs (id, version, manifest, locales, templates, source, imported_by,
                                  draft_id, store)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            prefab.id,
            prefab.version,
            prefab.manifest,
            prefab.locales,
            prefab.templates,
            prefab.source,
            prefab.imported_by,
            prefab.draft_id,
            prefab.store,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Versions d'essai qu'aucun serveur n'utilise plus : (id, version).
    pub async fn unused_trials(
        conn: &mut PgConnection,
    ) -> Result<Vec<(String, String)>, sqlx::Error> {
        let rows = sqlx::query!(
            "SELECT p.id, p.version FROM prefabs p
             WHERE p.source = 'trial'
               AND NOT EXISTS (SELECT 1 FROM servers s
                               WHERE s.prefab_id = p.id AND s.prefab_version = p.version)
             ORDER BY p.id, p.version"
        )
        .fetch_all(conn)
        .await?;
        Ok(rows.into_iter().map(|row| (row.id, row.version)).collect())
    }

    /// Serveurs par version : (id, version, nombre).
    pub async fn server_counts(
        conn: &mut PgConnection,
    ) -> Result<Vec<(String, String, i64)>, sqlx::Error> {
        let rows = sqlx::query!(
            r#"SELECT prefab_id, prefab_version, count(*) AS "count!"
               FROM servers GROUP BY prefab_id, prefab_version"#
        )
        .fetch_all(conn)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| (row.prefab_id, row.prefab_version, row.count))
            .collect())
    }

    /// Serveurs d'une version : (id, nom, état).
    pub async fn servers_of(
        conn: &mut PgConnection,
        id: &str,
        version: &str,
    ) -> Result<Vec<(uuid::Uuid, String, String)>, sqlx::Error> {
        let rows = sqlx::query!(
            "SELECT id, name, state FROM servers
             WHERE prefab_id = $1 AND prefab_version = $2 ORDER BY name",
            id,
            version,
        )
        .fetch_all(conn)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| (row.id, row.name, row.state))
            .collect())
    }

    /// Verrouille la version pour la supprimer ; `false` si elle n'existe pas.
    pub async fn lock(
        conn: &mut PgConnection,
        id: &str,
        version: &str,
    ) -> Result<bool, sqlx::Error> {
        Ok(sqlx::query!(
            "SELECT id FROM prefabs WHERE id = $1 AND version = $2 FOR UPDATE",
            id,
            version,
        )
        .fetch_optional(conn)
        .await?
        .is_some())
    }

    /// Supprime la version et ses images. La base refuse si un serveur l'utilise encore.
    pub async fn delete(
        conn: &mut PgConnection,
        id: &str,
        version: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "DELETE FROM prefabs WHERE id = $1 AND version = $2",
            id,
            version
        )
        .execute(conn)
        .await?;
        Ok(())
    }
}
