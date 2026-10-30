//! Persistance du serveur d'autorisation : clients, demandes, accords et jetons (hachés).

use std::time::Duration;

use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use super::domain::ValidClient;

#[derive(Debug, Clone)]
pub struct ClientRecord {
    pub id: Uuid,
    pub client_id: String,
    pub registration: String,
    pub name: String,
    pub uri: Option<String>,
    pub redirect_uris: Vec<String>,
    pub fetched_at: Option<OffsetDateTime>,
}

/// Demande d'autorisation et son client, pour la page de consentement.
#[derive(Debug, Clone)]
pub struct RequestRecord {
    pub id: Uuid,
    pub client_uuid: Uuid,
    pub client_id: String,
    pub registration: String,
    pub client_name: String,
    pub client_uri: Option<String>,
    pub redirect_uri: String,
    pub state: Option<String>,
    pub scopes: Vec<String>,
    pub expires_at: OffsetDateTime,
    pub granted: bool,
}

pub struct NewRequest<'a> {
    pub id: Uuid,
    pub client: Uuid,
    pub redirect_uri: &'a str,
    pub state: Option<&'a str>,
    pub code_challenge: &'a str,
    pub scopes: &'a [String],
}

/// Code échangé : ce qu'il faut revérifier avant d'émettre les jetons.
#[derive(Debug, Clone)]
pub struct RedeemedCode {
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub grant_id: Uuid,
    pub expired: bool,
}

#[derive(Debug, Clone)]
pub struct GrantRecord {
    pub id: Uuid,
    pub user_id: Uuid,
    pub client_id: String,
    pub client_name: String,
    pub registration: String,
    pub scopes: Vec<String>,
    pub created_at: OffsetDateTime,
    pub last_used_at: Option<OffsetDateTime>,
}

/// Jeton de rafraîchissement présenté.
#[derive(Debug, Clone)]
pub struct RefreshRecord {
    pub grant_id: Uuid,
    pub client_id: String,
    pub user_id: Uuid,
    pub expired: bool,
    pub consumed: bool,
}

/// Jeton d'accès valide et l'accord qui le porte.
#[derive(Debug, Clone)]
pub struct AccessRecord {
    pub grant_id: Uuid,
    pub user_id: Uuid,
    pub client_id: String,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Access,
    Refresh,
}

impl TokenKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Access => "access",
            Self::Refresh => "refresh",
        }
    }
}

pub struct OAuthRepository;

impl OAuthRepository {
    pub async fn client(
        conn: &mut PgConnection,
        client_id: &str,
    ) -> Result<Option<ClientRecord>, sqlx::Error> {
        sqlx::query_as!(
            ClientRecord,
            "SELECT id, client_id, registration, name, uri, redirect_uris, fetched_at
             FROM oauth_clients WHERE client_id = $1",
            client_id,
        )
        .fetch_optional(conn)
        .await
    }

    /// Document de métadonnées lu (ou relu) : le client est créé ou mis à jour.
    pub async fn save_metadata_client(
        conn: &mut PgConnection,
        client_id: &str,
        client: &ValidClient,
    ) -> Result<ClientRecord, sqlx::Error> {
        sqlx::query_as!(
            ClientRecord,
            "INSERT INTO oauth_clients (id, client_id, registration, name, uri, redirect_uris, fetched_at)
             VALUES ($1, $2, 'metadata', $3, $4, $5, now())
             ON CONFLICT (client_id) DO UPDATE
                SET name = EXCLUDED.name, uri = EXCLUDED.uri,
                    redirect_uris = EXCLUDED.redirect_uris, fetched_at = EXCLUDED.fetched_at
             RETURNING id, client_id, registration, name, uri, redirect_uris, fetched_at",
            Uuid::now_v7(),
            client_id,
            client.name,
            client.uri,
            &client.redirect_uris,
        )
        .fetch_one(conn)
        .await
    }

    pub async fn insert_dynamic_client(
        conn: &mut PgConnection,
        id: Uuid,
        client_id: &str,
        client: &ValidClient,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO oauth_clients (id, client_id, registration, name, uri, redirect_uris)
             VALUES ($1, $2, 'dynamic', $3, $4, $5)",
            id,
            client_id,
            client.name,
            client.uri,
            &client.redirect_uris,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Oublie les clients dynamiques jamais accordés, et les demandes expirées.
    pub async fn purge(conn: &mut PgConnection, unused_for: Duration) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "DELETE FROM oauth_requests
             WHERE (code_hash IS NULL AND expires_at < now()) OR code_expires_at < now()"
        )
        .execute(&mut *conn)
        .await?;
        sqlx::query!(
            "DELETE FROM oauth_clients c
             WHERE c.registration = 'dynamic'
               AND c.created_at < now() - make_interval(secs => $1)
               AND NOT EXISTS (SELECT 1 FROM oauth_grants g WHERE g.client_id = c.id)
               AND NOT EXISTS (SELECT 1 FROM oauth_requests r WHERE r.client_id = c.id)",
            unused_for.as_secs_f64(),
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn insert_request(
        conn: &mut PgConnection,
        request: &NewRequest<'_>,
        ttl: Duration,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO oauth_requests (id, client_id, redirect_uri, state, code_challenge, scopes, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, now() + make_interval(secs => $7))",
            request.id,
            request.client,
            request.redirect_uri,
            request.state,
            request.code_challenge,
            request.scopes,
            ttl.as_secs_f64(),
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Demande encore ouverte au consentement (ni expirée, ni déjà accordée).
    pub async fn pending_request(
        conn: &mut PgConnection,
        id: Uuid,
        lock: bool,
    ) -> Result<Option<RequestRecord>, sqlx::Error> {
        let record = if lock {
            sqlx::query_as!(
                RequestRecord,
                r#"SELECT r.id, c.id AS client_uuid, c.client_id, c.registration, c.name AS client_name,
                          c.uri AS client_uri, r.redirect_uri, r.state, r.scopes, r.expires_at,
                          (r.grant_id IS NOT NULL) AS "granted!"
                   FROM oauth_requests r JOIN oauth_clients c ON c.id = r.client_id
                   WHERE r.id = $1 AND r.expires_at > now() AND r.grant_id IS NULL
                   FOR UPDATE OF r"#,
                id,
            )
            .fetch_optional(conn)
            .await?
        } else {
            sqlx::query_as!(
                RequestRecord,
                r#"SELECT r.id, c.id AS client_uuid, c.client_id, c.registration, c.name AS client_name,
                          c.uri AS client_uri, r.redirect_uri, r.state, r.scopes, r.expires_at,
                          (r.grant_id IS NOT NULL) AS "granted!"
                   FROM oauth_requests r JOIN oauth_clients c ON c.id = r.client_id
                   WHERE r.id = $1 AND r.expires_at > now() AND r.grant_id IS NULL"#,
                id,
            )
            .fetch_optional(conn)
            .await?
        };
        Ok(record)
    }

    pub async fn set_code(
        conn: &mut PgConnection,
        request: Uuid,
        grant: Uuid,
        code_hash: &str,
        ttl: Duration,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE oauth_requests
             SET grant_id = $2, code_hash = $3, code_expires_at = now() + make_interval(secs => $4)
             WHERE id = $1",
            request,
            grant,
            code_hash,
            ttl.as_secs_f64(),
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn delete_request(conn: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query!("DELETE FROM oauth_requests WHERE id = $1", id)
            .execute(conn)
            .await?;
        Ok(())
    }

    /// Échange d'un code : la demande est effacée, le code ne sert qu'une fois.
    pub async fn redeem_code(
        conn: &mut PgConnection,
        code_hash: &str,
    ) -> Result<Option<RedeemedCode>, sqlx::Error> {
        sqlx::query_as!(
            RedeemedCode,
            r#"WITH taken AS (
                   DELETE FROM oauth_requests WHERE code_hash = $1
                   RETURNING client_id, redirect_uri, code_challenge, grant_id, code_expires_at
               )
               SELECT c.client_id, t.redirect_uri, t.code_challenge, t.grant_id AS "grant_id!",
                      (t.code_expires_at < now()) AS "expired!"
               FROM taken t JOIN oauth_clients c ON c.id = t.client_id"#,
            code_hash,
        )
        .fetch_optional(conn)
        .await
    }

    /// Nouvel accord, qui remplace celui du même admin pour le même client. Rend l'identifiant
    /// de l'accord remplacé.
    pub async fn replace_grant(
        conn: &mut PgConnection,
        id: Uuid,
        user_id: Uuid,
        client: Uuid,
        scopes: &[String],
    ) -> Result<Option<Uuid>, sqlx::Error> {
        let replaced = sqlx::query_scalar!(
            "DELETE FROM oauth_grants WHERE user_id = $1 AND client_id = $2 RETURNING id",
            user_id,
            client,
        )
        .fetch_optional(&mut *conn)
        .await?;
        sqlx::query!(
            "INSERT INTO oauth_grants (id, user_id, client_id, scopes) VALUES ($1, $2, $3, $4)",
            id,
            user_id,
            client,
            scopes,
        )
        .execute(conn)
        .await?;
        Ok(replaced)
    }

    /// Admin et droits d'un accord.
    pub async fn grant_owner(
        conn: &mut PgConnection,
        id: Uuid,
    ) -> Result<Option<(Uuid, Vec<String>)>, sqlx::Error> {
        let row = sqlx::query!("SELECT user_id, scopes FROM oauth_grants WHERE id = $1", id)
            .fetch_optional(conn)
            .await?;
        Ok(row.map(|r| (r.user_id, r.scopes)))
    }

    pub async fn grants_of(
        conn: &mut PgConnection,
        user_id: Uuid,
    ) -> Result<Vec<GrantRecord>, sqlx::Error> {
        sqlx::query_as!(
            GrantRecord,
            "SELECT g.id, g.user_id, c.client_id, c.name AS client_name, c.registration, g.scopes,
                    g.created_at, g.last_used_at
             FROM oauth_grants g JOIN oauth_clients c ON c.id = g.client_id
             WHERE g.user_id = $1 ORDER BY g.created_at DESC",
            user_id,
        )
        .fetch_all(conn)
        .await
    }

    /// Révoque un accord (ses jetons partent avec lui). Rend son client.
    pub async fn delete_grant(
        conn: &mut PgConnection,
        id: Uuid,
        user_id: Option<Uuid>,
    ) -> Result<Option<String>, sqlx::Error> {
        sqlx::query_scalar!(
            "WITH gone AS (
                 DELETE FROM oauth_grants WHERE id = $1 AND ($2::uuid IS NULL OR user_id = $2)
                 RETURNING client_id
             )
             SELECT c.client_id FROM gone JOIN oauth_clients c ON c.id = gone.client_id",
            id,
            user_id,
        )
        .fetch_optional(conn)
        .await
    }

    pub async fn insert_token(
        conn: &mut PgConnection,
        token_hash: &str,
        grant: Uuid,
        kind: TokenKind,
        ttl: Duration,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO oauth_tokens (token_hash, grant_id, kind, expires_at)
             VALUES ($1, $2, $3, now() + make_interval(secs => $4))",
            token_hash,
            grant,
            kind.as_str(),
            ttl.as_secs_f64(),
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Jetons expirés d'un accord, effacés à chaque émission.
    pub async fn purge_tokens(conn: &mut PgConnection, grant: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "DELETE FROM oauth_tokens WHERE grant_id = $1 AND expires_at < now()",
            grant
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn refresh_token(
        conn: &mut PgConnection,
        token_hash: &str,
    ) -> Result<Option<RefreshRecord>, sqlx::Error> {
        sqlx::query_as!(
            RefreshRecord,
            r#"SELECT t.grant_id, c.client_id, g.user_id,
                      (t.expires_at < now()) AS "expired!",
                      (t.consumed_at IS NOT NULL) AS "consumed!"
               FROM oauth_tokens t
               JOIN oauth_grants g ON g.id = t.grant_id
               JOIN oauth_clients c ON c.id = g.client_id
               WHERE t.token_hash = $1 AND t.kind = 'refresh'
               FOR UPDATE OF t"#,
            token_hash,
        )
        .fetch_optional(conn)
        .await
    }

    pub async fn consume(conn: &mut PgConnection, token_hash: &str) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE oauth_tokens SET consumed_at = now() WHERE token_hash = $1",
            token_hash
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Efface un jeton ; rend son accord, son client et sa nature.
    pub async fn delete_token(
        conn: &mut PgConnection,
        token_hash: &str,
    ) -> Result<Option<(Uuid, String, bool)>, sqlx::Error> {
        let row = sqlx::query!(
            r#"WITH gone AS (DELETE FROM oauth_tokens WHERE token_hash = $1 RETURNING grant_id, kind)
               SELECT gone.grant_id, c.client_id, (gone.kind = 'refresh') AS "refresh!"
               FROM gone JOIN oauth_grants g ON g.id = gone.grant_id
               JOIN oauth_clients c ON c.id = g.client_id"#,
            token_hash,
        )
        .fetch_optional(conn)
        .await?;
        Ok(row.map(|r| (r.grant_id, r.client_id, r.refresh)))
    }

    /// Jeton d'accès valide ; marque l'accord comme utilisé.
    pub async fn access_token(
        conn: &mut PgConnection,
        token_hash: &str,
    ) -> Result<Option<AccessRecord>, sqlx::Error> {
        sqlx::query_as!(
            AccessRecord,
            "WITH used AS (
                 UPDATE oauth_grants g SET last_used_at = now()
                 FROM oauth_tokens t
                 WHERE t.token_hash = $1 AND t.kind = 'access' AND t.expires_at > now()
                   AND g.id = t.grant_id
                 RETURNING g.id, g.user_id, g.client_id, g.scopes
             )
             SELECT used.id AS grant_id, used.user_id, c.client_id, used.scopes
             FROM used JOIN oauth_clients c ON c.id = used.client_id",
            token_hash,
        )
        .fetch_optional(conn)
        .await
    }
}
