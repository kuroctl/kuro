//! Adaptateurs du domaine `servers`.

mod backups;
mod content;
mod schedules;
mod tickets;

pub use backups::{BackupRecord, BackupRepository};
pub use content::{ContentPackage, ContentRecord, ContentRepository, NewContent};
pub use schedules::{RunRecord, ScheduleFields, ScheduleRecord, ScheduleRepository, StepRecord};
pub use tickets::{RedeemedTicket, TicketRepository};

use serde_json::Value;
use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

/// Serveur stocké. `secrets` : réglages `secret` chiffrés, jamais renvoyés par l'API.
#[derive(Debug, Clone)]
pub struct ServerRecord {
    pub id: Uuid,
    pub name: String,
    pub node_id: Uuid,
    pub node_name: String,
    pub node_status: String,
    pub prefab_id: String,
    pub prefab_version: String,
    pub settings: Value,
    pub secrets: Option<Vec<u8>>,
    pub memory_mb: i32,
    pub cpu_percent: i32,
    pub disk_mb: i32,
    /// Sauvegardes gardées (ADR-0018) ; 0 les désactive.
    pub backup_limit: i32,
    pub state: String,
    pub state_reason: Option<String>,
    /// Suspendu par un admin (ADR-0019).
    pub suspended_at: Option<OffsetDateTime>,
    /// Espace occupé (volumes et sauvegardes), mesuré par le daemon.
    pub disk_used_bytes: Option<i64>,
    pub disk_measured_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    /// Serveur d'essai du Studio : brouillon essayé (effacé si le brouillon disparaît).
    pub trial_draft_id: Option<Uuid>,
    /// Fin d'un essai : le serveur est supprimé passé cette date.
    pub expires_at: Option<OffsetDateTime>,
    /// Nom dans la zone DNS (ADR-0026).
    pub dns_label: Option<String>,
    /// Identifiant du site web (`<web_id>.<domaine des sites>`, ADR-0038) ; attribué à la
    /// création d'un serveur dont le prefab a un port web.
    pub web_id: Option<String>,
    /// État du proxy et du certificat du site, rapporté par le daemon (`WebStatus` du protocole).
    pub web_status: Option<Value>,
}

/// Nouveau serveur, inséré en état `deploying`.
#[derive(Debug)]
pub struct NewServerRow<'a> {
    pub id: Uuid,
    pub name: &'a str,
    pub node_id: Uuid,
    pub prefab_id: &'a str,
    pub prefab_version: &'a str,
    pub settings: &'a Value,
    pub secrets: Option<&'a [u8]>,
    pub memory_mb: i32,
    pub cpu_percent: i32,
    pub disk_mb: i32,
    /// Démarrer le serveur dès la fin de son premier déploiement (installation comprise).
    pub start_when_installed: bool,
    pub backup_limit: i32,
    /// Identifiant du site (ADR-0038), pour un prefab à port web.
    pub web_id: Option<&'a str>,
}

/// Colonnes légères d'un serveur, pour chercher, filtrer, trier et compter sans charger ses
/// réglages ni ses secrets.
#[derive(Debug, Clone)]
pub struct ServerIndexRow {
    pub id: Uuid,
    pub name: String,
    pub node_id: Uuid,
    pub node_name: String,
    pub prefab_id: String,
    pub prefab_version: String,
    pub state: String,
    pub suspended: bool,
    pub memory_mb: i32,
    pub cpu_percent: i32,
    pub disk_mb: i32,
    pub created_at: OffsetDateTime,
}

#[derive(Debug)]
pub struct ServerRepository;

/// Changements de contenu d'un serveur en attente de son prochain démarrage.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PendingChanges {
    pub server_id: Uuid,
    pub restart_required: bool,
    /// Contenus ajoutés, activés, désactivés ou modifiés.
    pub contents: i64,
    pub removals: i64,
}

impl ServerRepository {
    /// Serveurs par nom, `limit` au plus (`None` : tous) à partir de `offset`.
    pub async fn list(
        conn: &mut PgConnection,
        limit: Option<i64>,
        offset: i64,
    ) -> Result<Vec<ServerRecord>, sqlx::Error> {
        sqlx::query_as!(
            ServerRecord,
            r#"SELECT s.id, s.name, s.node_id, n.name AS node_name, n.status AS node_status,
                      s.prefab_id, s.prefab_version, s.settings, s.secrets,
                      s.memory_mb, s.cpu_percent, s.disk_mb, s.backup_limit, s.state, s.state_reason,
                      s.suspended_at, s.disk_used_bytes, s.disk_measured_at,
                      s.created_at, s.updated_at, s.trial_draft_id, s.expires_at, s.dns_label,
                      s.web_id, s.web_status
               FROM servers s JOIN nodes n ON n.id = s.node_id
               ORDER BY s.name, s.id
               LIMIT $1 OFFSET $2"#,
            limit,
            offset,
        )
        .fetch_all(conn)
        .await
    }

    /// Serveurs d'un prefab, toutes versions confondues.
    pub async fn of_prefab(
        conn: &mut PgConnection,
        prefab_id: &str,
    ) -> Result<Vec<ServerRecord>, sqlx::Error> {
        sqlx::query_as!(
            ServerRecord,
            r#"SELECT s.id, s.name, s.node_id, n.name AS node_name, n.status AS node_status,
                      s.prefab_id, s.prefab_version, s.settings, s.secrets,
                      s.memory_mb, s.cpu_percent, s.disk_mb, s.backup_limit, s.state, s.state_reason,
                      s.suspended_at, s.disk_used_bytes, s.disk_measured_at,
                      s.created_at, s.updated_at, s.trial_draft_id, s.expires_at, s.dns_label,
                      s.web_id, s.web_status
               FROM servers s JOIN nodes n ON n.id = s.node_id
               WHERE s.prefab_id = $1
               ORDER BY s.name, s.id"#,
            prefab_id,
        )
        .fetch_all(conn)
        .await
    }

    pub async fn index(conn: &mut PgConnection) -> Result<Vec<ServerIndexRow>, sqlx::Error> {
        sqlx::query_as!(
            ServerIndexRow,
            r#"SELECT s.id, s.name, s.node_id, n.name AS node_name, s.prefab_id, s.prefab_version,
                      s.state, (s.suspended_at IS NOT NULL) AS "suspended!",
                      s.memory_mb, s.cpu_percent, s.disk_mb, s.created_at
               FROM servers s JOIN nodes n ON n.id = s.node_id"#,
        )
        .fetch_all(conn)
        .await
    }

    /// Serveurs complets d'une liste d'identifiants (ordre quelconque).
    pub async fn by_ids(
        conn: &mut PgConnection,
        ids: &[Uuid],
    ) -> Result<Vec<ServerRecord>, sqlx::Error> {
        sqlx::query_as!(
            ServerRecord,
            r#"SELECT s.id, s.name, s.node_id, n.name AS node_name, n.status AS node_status,
                      s.prefab_id, s.prefab_version, s.settings, s.secrets,
                      s.memory_mb, s.cpu_percent, s.disk_mb, s.backup_limit, s.state, s.state_reason,
                      s.suspended_at, s.disk_used_bytes, s.disk_measured_at,
                      s.created_at, s.updated_at, s.trial_draft_id, s.expires_at, s.dns_label,
                      s.web_id, s.web_status
               FROM servers s JOIN nodes n ON n.id = s.node_id
               WHERE s.id = ANY($1)"#,
            ids,
        )
        .fetch_all(conn)
        .await
    }

    pub async fn get(
        conn: &mut PgConnection,
        id: Uuid,
    ) -> Result<Option<ServerRecord>, sqlx::Error> {
        sqlx::query_as!(
            ServerRecord,
            r#"SELECT s.id, s.name, s.node_id, n.name AS node_name, n.status AS node_status,
                      s.prefab_id, s.prefab_version, s.settings, s.secrets,
                      s.memory_mb, s.cpu_percent, s.disk_mb, s.backup_limit, s.state, s.state_reason,
                      s.suspended_at, s.disk_used_bytes, s.disk_measured_at,
                      s.created_at, s.updated_at, s.trial_draft_id, s.expires_at, s.dns_label,
                      s.web_id, s.web_status
               FROM servers s JOIN nodes n ON n.id = s.node_id
               WHERE s.id = $1"#,
            id,
        )
        .fetch_optional(conn)
        .await
    }

    /// Fait du serveur l'essai d'un brouillon, jusqu'à `expires_at`.
    pub async fn mark_trial(
        conn: &mut PgConnection,
        id: Uuid,
        draft: Uuid,
        expires_at: OffsetDateTime,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE servers SET trial_draft_id = $2, expires_at = $3 WHERE id = $1",
            id,
            draft,
            expires_at,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Essais en cours d'un brouillon (hors suppression déjà demandée).
    pub async fn trials_of(conn: &mut PgConnection, draft: Uuid) -> Result<Vec<Uuid>, sqlx::Error> {
        sqlx::query_scalar!(
            "SELECT id FROM servers WHERE trial_draft_id = $1 AND state <> 'deleting' ORDER BY id",
            draft,
        )
        .fetch_all(conn)
        .await
    }

    /// Essais arrivés à expiration (hors suppression déjà demandée).
    pub async fn expired_trials(
        conn: &mut PgConnection,
        now: OffsetDateTime,
    ) -> Result<Vec<Uuid>, sqlx::Error> {
        sqlx::query_scalar!(
            "SELECT id FROM servers WHERE expires_at <= $1 AND state <> 'deleting' ORDER BY id",
            now,
        )
        .fetch_all(conn)
        .await
    }

    /// Verrouille le serveur pour la durée de la transaction.
    pub async fn lock(conn: &mut PgConnection, id: Uuid) -> Result<bool, sqlx::Error> {
        Ok(
            sqlx::query!("SELECT id FROM servers WHERE id = $1 FOR UPDATE", id)
                .fetch_optional(conn)
                .await?
                .is_some(),
        )
    }

    pub async fn insert(conn: &mut PgConnection, row: NewServerRow<'_>) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO servers (id, name, node_id, prefab_id, prefab_version, settings, secrets,
                                  memory_mb, cpu_percent, disk_mb, start_when_installed,
                                  backup_limit, web_id, state)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, 'deploying')",
            row.id,
            row.name,
            row.node_id,
            row.prefab_id,
            row.prefab_version,
            row.settings,
            row.secrets,
            row.memory_mb,
            row.cpu_percent,
            row.disk_mb,
            row.start_when_installed,
            row.backup_limit,
            row.web_id,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Donne son identifiant de site à un serveur dont le prefab devient web (ADR-0038).
    pub async fn set_web_id(
        conn: &mut PgConnection,
        id: Uuid,
        web_id: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE servers SET web_id = $2, updated_at = now() WHERE id = $1",
            id,
            web_id,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn update_settings(
        conn: &mut PgConnection,
        id: Uuid,
        settings: &Value,
        secrets: Option<&[u8]>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE servers SET settings = $2, secrets = $3, updated_at = now() WHERE id = $1",
            id,
            settings,
            secrets,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Passage à une autre version du prefab, réglages migrés compris (ADR-0024).
    pub async fn upgrade(
        conn: &mut PgConnection,
        id: Uuid,
        version: &str,
        settings: &Value,
        secrets: Option<&[u8]>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE servers SET prefab_version = $2, settings = $3, secrets = $4, updated_at = now()
             WHERE id = $1",
            id,
            version,
            settings,
            secrets,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Démarrer le serveur dès la fin de sa prochaine installation.
    pub async fn set_start_when_installed(
        conn: &mut PgConnection,
        id: Uuid,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE servers SET start_when_installed = true WHERE id = $1",
            id
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Nouvelles ressources (ADR-0019).
    pub async fn update_resources(
        conn: &mut PgConnection,
        id: Uuid,
        memory_mb: i32,
        cpu_percent: i32,
        disk_mb: i32,
        backup_limit: i32,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE servers SET memory_mb = $2, cpu_percent = $3, disk_mb = $4, backup_limit = $5,
                                updated_at = now()
             WHERE id = $1",
            id,
            memory_mb,
            cpu_percent,
            disk_mb,
            backup_limit,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Suspend (`true`) ou réactive le serveur.
    pub async fn set_suspended(
        conn: &mut PgConnection,
        id: Uuid,
        suspended: bool,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE servers SET suspended_at = CASE WHEN $2 THEN now() END, updated_at = now()
             WHERE id = $1",
            id,
            suspended,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Espace disque rapporté par un nœud, pour l'un de ses serveurs seulement.
    pub async fn record_disk_if_owned(
        conn: &mut PgConnection,
        id: Uuid,
        node_id: Uuid,
        used_bytes: i64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE servers SET disk_used_bytes = $3, disk_measured_at = now()
             WHERE id = $1 AND node_id = $2",
            id,
            node_id,
            used_bytes,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn set_state(
        conn: &mut PgConnection,
        id: Uuid,
        state: &str,
        reason: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE servers SET state = $2, state_reason = $3, updated_at = now() WHERE id = $1",
            id,
            state,
            reason,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Consomme le démarrage demandé à la création, pour un serveur du nœud : vrai une seule
    /// fois, même si l'état est rapporté deux fois.
    pub async fn take_start_when_installed(
        conn: &mut PgConnection,
        id: Uuid,
        node_id: Uuid,
    ) -> Result<bool, sqlx::Error> {
        let updated = sqlx::query!(
            "UPDATE servers SET start_when_installed = false
             WHERE id = $1 AND node_id = $2 AND start_when_installed",
            id,
            node_id,
        )
        .execute(conn)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    /// Changement de contenu en attente du prochain démarrage : contenus concernés, et retraits
    /// comptés (leur ligne n'existe plus).
    pub async fn defer_changes(
        conn: &mut PgConnection,
        server_id: Uuid,
        contents: &[Uuid],
        removed: i32,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE servers SET restart_required = true,
                    pending_removals = pending_removals + $2
             WHERE id = $1",
            server_id,
            removed,
        )
        .execute(&mut *conn)
        .await?;
        sqlx::query!(
            "UPDATE server_content SET pending = true WHERE server_id = $1 AND id = ANY($2)",
            server_id,
            contents,
        )
        .execute(&mut *conn)
        .await?;
        Ok(())
    }

    /// Le serveur démarre : ce qui l'attendait est appliqué.
    pub async fn clear_pending(
        conn: &mut PgConnection,
        server_id: Uuid,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE servers SET restart_required = false, pending_removals = 0
             WHERE id = $1 AND restart_required",
            server_id,
        )
        .execute(&mut *conn)
        .await?;
        sqlx::query!(
            "UPDATE server_content SET pending = false WHERE server_id = $1 AND pending",
            server_id,
        )
        .execute(&mut *conn)
        .await?;
        Ok(())
    }

    /// Changements en attente des serveurs donnés : (serveur, redémarrage requis, contenus en
    /// attente, retraits).
    pub async fn pending(
        conn: &mut PgConnection,
        ids: &[Uuid],
    ) -> Result<Vec<PendingChanges>, sqlx::Error> {
        let rows = sqlx::query!(
            r#"SELECT s.id, s.restart_required, s.pending_removals,
                      (SELECT count(*) FROM server_content c WHERE c.server_id = s.id AND c.pending)
                        AS "pending_contents!"
               FROM servers s WHERE s.id = ANY($1)"#,
            ids,
        )
        .fetch_all(conn)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| PendingChanges {
                server_id: r.id,
                restart_required: r.restart_required,
                contents: r.pending_contents,
                removals: i64::from(r.pending_removals),
            })
            .collect())
    }

    /// État rapporté par un nœud, pour l'un de ses serveurs seulement. Un serveur en
    /// suppression ne revient qu'en `failed` (la suppression a échoué).
    pub async fn set_state_if_owned(
        conn: &mut PgConnection,
        id: Uuid,
        node_id: Uuid,
        state: &str,
        reason: Option<&str>,
    ) -> Result<bool, sqlx::Error> {
        let updated = sqlx::query!(
            "UPDATE servers SET state = $3, state_reason = $4, updated_at = now()
             WHERE id = $1 AND node_id = $2 AND (state <> 'deleting' OR $3 = 'failed')",
            id,
            node_id,
            state,
            reason,
        )
        .execute(conn)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    /// État du site web rapporté par le nœud du serveur (ADR-0038).
    pub async fn set_web_status_if_owned(
        conn: &mut PgConnection,
        id: Uuid,
        node_id: Uuid,
        status: &Value,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE servers SET web_status = $3 WHERE id = $1 AND node_id = $2",
            id,
            node_id,
            status,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Suppression confirmée par le nœud du serveur : ses ports sont libérés, puis il disparaît.
    pub async fn delete_if_owned(
        conn: &mut PgConnection,
        id: Uuid,
        node_id: Uuid,
    ) -> Result<bool, sqlx::Error> {
        let mut tx = sqlx::Connection::begin(&mut *conn).await?;
        let owned = sqlx::query!(
            "SELECT id FROM servers WHERE id = $1 AND node_id = $2 FOR UPDATE",
            id,
            node_id
        )
        .fetch_optional(&mut *tx)
        .await?
        .is_some();
        if owned {
            sqlx::query!(
                "UPDATE allocations SET server_id = NULL, port_name = NULL WHERE server_id = $1",
                id
            )
            .execute(&mut *tx)
            .await?;
            sqlx::query!("DELETE FROM servers WHERE id = $1", id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(owned)
    }

    pub async fn ids_of_node(
        conn: &mut PgConnection,
        node_id: Uuid,
    ) -> Result<Vec<Uuid>, sqlx::Error> {
        let rows = sqlx::query!(
            "SELECT id FROM servers WHERE node_id = $1 ORDER BY id",
            node_id
        )
        .fetch_all(conn)
        .await?;
        Ok(rows.into_iter().map(|r| r.id).collect())
    }
}
