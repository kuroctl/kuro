//! Contenu installé des serveurs (ADR-0020) et liens de dépendance.

use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ContentRecord {
    pub id: Uuid,
    pub server_id: Uuid,
    pub slot: String,
    pub source: String,
    pub project_id: Option<String>,
    pub version_id: Option<String>,
    pub name: String,
    pub version_name: Option<String>,
    pub icon_url: Option<String>,
    pub file_name: String,
    pub sha512: Option<String>,
    pub size_bytes: Option<i64>,
    pub enabled: bool,
    pub state: String,
    pub failure: Option<String>,
    pub modpack_id: Option<Uuid>,
    pub operation_id: Option<Uuid>,
    pub replaces: Option<Uuid>,
    /// `file` ou `modpack` (sans fichier propre ; ses fichiers le référencent).
    pub kind: String,
    /// Valeurs saisies pour les champs de l'emplacement (ADR-0030) : champ → liste.
    pub field_values: serde_json::Value,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// Paquet extrait d'un contenu (mode `extract`) : dossier de sa route, nom du dossier créé,
/// description lue par le nœud.
#[derive(Debug, Clone, PartialEq)]
pub struct ContentPackage {
    pub dir: String,
    pub name: String,
    pub description: serde_json::Value,
}

impl ContentPackage {
    /// Chemin du dossier du paquet dans le conteneur.
    pub fn path(&self) -> String {
        format!("{}/{}", self.dir.trim_end_matches('/'), self.name)
    }
}

/// Nouveau contenu, inséré en état `installing`.
#[derive(Debug)]
pub struct NewContent<'a> {
    pub id: Uuid,
    pub server_id: Uuid,
    pub slot: &'a str,
    pub source: &'a str,
    pub project_id: Option<&'a str>,
    pub version_id: Option<&'a str>,
    pub name: &'a str,
    pub version_name: Option<&'a str>,
    pub icon_url: Option<&'a str>,
    pub file_name: &'a str,
    pub sha512: Option<&'a str>,
    /// Empreinte des fichiers CurseForge.
    pub sha1: Option<&'a str>,
    /// Valeurs saisies pour les champs de l'emplacement (ADR-0030).
    pub field_values: &'a serde_json::Value,
    pub size_bytes: Option<i64>,
    pub operation_id: Option<Uuid>,
    pub replaces: Option<Uuid>,
    pub kind: &'a str,
    pub modpack_id: Option<Uuid>,
    /// `installing` (le nœud télécharge) ou `installed` (fichier déjà déposé : envoi).
    pub state: &'a str,
}

#[derive(Debug)]
pub struct ContentRepository;

impl ContentRepository {
    pub async fn of_slot(
        conn: &mut PgConnection,
        server_id: Uuid,
        slot: &str,
    ) -> Result<Vec<ContentRecord>, sqlx::Error> {
        sqlx::query_as!(
            ContentRecord,
            "SELECT id, server_id, slot, source, project_id, version_id, name, version_name,
                    icon_url, file_name, sha512, size_bytes, enabled, state, failure, modpack_id,
                    operation_id, replaces, kind, field_values, created_at, updated_at
             FROM server_content WHERE server_id = $1 AND slot = $2
             ORDER BY lower(name), created_at",
            server_id,
            slot,
        )
        .fetch_all(conn)
        .await
    }

    pub async fn get(
        conn: &mut PgConnection,
        server_id: Uuid,
        id: Uuid,
    ) -> Result<Option<ContentRecord>, sqlx::Error> {
        sqlx::query_as!(
            ContentRecord,
            "SELECT id, server_id, slot, source, project_id, version_id, name, version_name,
                    icon_url, file_name, sha512, size_bytes, enabled, state, failure, modpack_id,
                    operation_id, replaces, kind, field_values, created_at, updated_at
             FROM server_content WHERE server_id = $1 AND id = $2",
            server_id,
            id,
        )
        .fetch_optional(conn)
        .await
    }

    pub async fn insert(conn: &mut PgConnection, row: NewContent<'_>) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO server_content (id, server_id, slot, source, project_id, version_id, name,
                                         version_name, icon_url, file_name, sha512, size_bytes,
                                         state, operation_id, replaces, kind, modpack_id, sha1,
                                         field_values)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $15, $13, $14, $16, $17,
                     $18, $19)",
            row.id,
            row.server_id,
            row.slot,
            row.source,
            row.project_id,
            row.version_id,
            row.name,
            row.version_name,
            row.icon_url,
            row.file_name,
            row.sha512,
            row.size_bytes,
            row.operation_id,
            row.replaces,
            row.state,
            row.kind,
            row.modpack_id,
            row.sha1,
            row.field_values,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Valeurs saisies d'un contenu (ADR-0030).
    pub async fn set_field_values(
        conn: &mut PgConnection,
        id: Uuid,
        values: &serde_json::Value,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE server_content SET field_values = $2, updated_at = now() WHERE id = $1",
            id,
            values,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Valeurs saisies des contenus actifs du serveur : (emplacement, valeurs), dans l'ordre
    /// d'ajout, pour les gabarits (`{{ content.mods.values.mod_ids }}`).
    pub async fn active_field_values(
        conn: &mut PgConnection,
        server_id: Uuid,
    ) -> Result<Vec<(String, serde_json::Value)>, sqlx::Error> {
        let rows = sqlx::query!(
            "SELECT slot, field_values FROM server_content
             WHERE server_id = $1 AND enabled AND state = 'installed' AND field_values <> '{}'::jsonb
             ORDER BY created_at, id",
            server_id,
        )
        .fetch_all(conn)
        .await?;
        Ok(rows.into_iter().map(|r| (r.slot, r.field_values)).collect())
    }

    /// Contenu en mode `reference` actif du serveur : (emplacement, identifiant chez la source),
    /// dans l'ordre d'ajout, pour les gabarits (`{{ content.mods.ids }}`).
    pub async fn reference_ids(
        conn: &mut PgConnection,
        server_id: Uuid,
    ) -> Result<Vec<(String, String)>, sqlx::Error> {
        let rows = sqlx::query!(
            r#"SELECT slot, project_id AS "project_id!" FROM server_content
               WHERE server_id = $1 AND kind = 'reference' AND enabled AND state = 'installed'
                     AND project_id IS NOT NULL
               ORDER BY created_at, id"#,
            server_id,
        )
        .fetch_all(conn)
        .await?;
        Ok(rows.into_iter().map(|r| (r.slot, r.project_id)).collect())
    }

    /// Contenus en attente du prochain démarrage du serveur.
    pub async fn pending_ids(
        conn: &mut PgConnection,
        server_id: Uuid,
    ) -> Result<Vec<Uuid>, sqlx::Error> {
        sqlx::query_scalar!(
            "SELECT id FROM server_content WHERE server_id = $1 AND pending",
            server_id
        )
        .fetch_all(conn)
        .await
    }

    /// Paquets extraits d'un contenu (mode `extract`, ADR-0030), dans l'ordre de l'archive ;
    /// remplace ceux qu'il avait.
    pub async fn set_packages(
        conn: &mut PgConnection,
        content_id: Uuid,
        packages: &[ContentPackage],
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "DELETE FROM server_content_packages WHERE content_id = $1",
            content_id
        )
        .execute(&mut *conn)
        .await?;
        for (position, package) in packages.iter().enumerate() {
            sqlx::query!(
                "INSERT INTO server_content_packages (content_id, position, dir, name, description)
                 VALUES ($1, $2, $3, $4, $5)",
                content_id,
                i32::try_from(position).unwrap_or(i32::MAX),
                package.dir,
                package.name,
                package.description,
            )
            .execute(&mut *conn)
            .await?;
        }
        Ok(())
    }

    /// Paquets des contenus du serveur : contenu → paquets, dans l'ordre de l'archive.
    pub async fn packages(
        conn: &mut PgConnection,
        server_id: Uuid,
    ) -> Result<Vec<(Uuid, ContentPackage)>, sqlx::Error> {
        let rows = sqlx::query!(
            "SELECT p.content_id, p.dir, p.name, p.description
             FROM server_content_packages p JOIN server_content c ON c.id = p.content_id
             WHERE c.server_id = $1
             ORDER BY c.created_at, c.id, p.position",
            server_id,
        )
        .fetch_all(conn)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                (
                    r.content_id,
                    ContentPackage {
                        dir: r.dir,
                        name: r.name,
                        description: r.description,
                    },
                )
            })
            .collect())
    }

    /// Descriptions des paquets actifs : (emplacement, description), dans l'ordre d'ajout, pour
    /// les fichiers d'inscription.
    pub async fn active_packages(
        conn: &mut PgConnection,
        server_id: Uuid,
    ) -> Result<Vec<(String, serde_json::Value)>, sqlx::Error> {
        let rows = sqlx::query!(
            "SELECT c.slot, p.description
             FROM server_content_packages p JOIN server_content c ON c.id = p.content_id
             WHERE c.server_id = $1 AND c.enabled AND c.state = 'installed'
             ORDER BY c.created_at, c.id, p.position",
            server_id,
        )
        .fetch_all(conn)
        .await?;
        Ok(rows.into_iter().map(|r| (r.slot, r.description)).collect())
    }

    pub async fn link(
        conn: &mut PgConnection,
        content_id: Uuid,
        required_by: Uuid,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO server_content_dependencies (content_id, required_by) VALUES ($1, $2)
             ON CONFLICT DO NOTHING",
            content_id,
            required_by,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Liens de dépendance des contenus du serveur : (contenu exigé, contenu qui l'exige).
    pub async fn links(
        conn: &mut PgConnection,
        server_id: Uuid,
    ) -> Result<Vec<(Uuid, Uuid)>, sqlx::Error> {
        let rows = sqlx::query!(
            "SELECT d.content_id, d.required_by FROM server_content_dependencies d
             JOIN server_content c ON c.id = d.content_id WHERE c.server_id = $1",
            server_id,
        )
        .fetch_all(conn)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| (r.content_id, r.required_by))
            .collect())
    }

    /// Reporte les liens d'un contenu remplacé (mise à jour) sur son successeur.
    pub async fn move_links(
        conn: &mut PgConnection,
        from: Uuid,
        to: Uuid,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO server_content_dependencies (content_id, required_by)
             SELECT $2, required_by FROM server_content_dependencies WHERE content_id = $1
             ON CONFLICT DO NOTHING",
            from,
            to,
        )
        .execute(&mut *conn)
        .await?;
        sqlx::query!(
            "INSERT INTO server_content_dependencies (content_id, required_by)
             SELECT content_id, $2 FROM server_content_dependencies WHERE required_by = $1
             ON CONFLICT DO NOTHING",
            from,
            to,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Fin d'une opération rapportée par un nœud, pour l'un de ses serveurs seulement : les
    /// contenus passent `installed` ou `failed`. Retourne ceux qui viennent de s'installer.
    pub async fn complete(
        conn: &mut PgConnection,
        node_id: Uuid,
        server_id: Uuid,
        operation_id: Uuid,
        failure: Option<&str>,
    ) -> Result<Vec<ContentRecord>, sqlx::Error> {
        sqlx::query_as!(
            ContentRecord,
            "UPDATE server_content c
             SET state = CASE WHEN $4::text IS NULL THEN 'installed' ELSE 'failed' END,
                 failure = $4, operation_id = NULL, updated_at = now()
             FROM servers s
             WHERE c.server_id = s.id AND s.id = $2 AND s.node_id = $1
               AND c.operation_id = $3 AND c.state = 'installing'
             RETURNING c.id, c.server_id, c.slot, c.source, c.project_id, c.version_id, c.name,
                       c.version_name, c.icon_url, c.file_name, c.sha512, c.size_bytes, c.enabled,
                       c.state, c.failure, c.modpack_id, c.operation_id, c.replaces, c.kind,
                       c.field_values, c.created_at, c.updated_at",
            node_id,
            server_id,
            operation_id,
            failure,
        )
        .fetch_all(conn)
        .await
    }

    pub async fn set_enabled(
        conn: &mut PgConnection,
        id: Uuid,
        enabled: bool,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE server_content SET enabled = $2, updated_at = now() WHERE id = $1",
            id,
            enabled,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn fail(
        conn: &mut PgConnection,
        ids: &[Uuid],
        reason: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE server_content SET state = 'failed', failure = $2, operation_id = NULL,
                    updated_at = now()
             WHERE id = ANY($1) AND state = 'installing'",
            ids,
            reason,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn delete(conn: &mut PgConnection, ids: &[Uuid]) -> Result<(), sqlx::Error> {
        sqlx::query!("DELETE FROM server_content WHERE id = ANY($1)", ids)
            .execute(conn)
            .await?;
        Ok(())
    }
}
