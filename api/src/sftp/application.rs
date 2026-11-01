//! Cas d'usage SFTP (ADR-0047) : clés SSH des comptes, coordonnées de connexion, et réponses aux
//! requêtes des nœuds (authentification, baux, audit des sessions).

use kuro_protocol::api::sftp::{SftpAuditAck, SftpDenial, SftpGrant, SftpLeaseResult, SftpLeases};
use kuro_protocol::daemon::sftp::{
    SftpAudit, SftpAuth, SftpCredential, SftpCredentialRef, SftpEvent, SftpEventKind, SftpLease,
    SftpLeaseRequest, SftpPrincipal,
};
use kuro_protocol::limits::{MAX_SFTP_AUDIT_BATCH, MAX_SSH_CLIENT_VERSION_BYTES, SFTP_LEASE_SECS};
use serde_json::json;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::domain::{self, KeyError, MAX_KEY_NAME_CHARS};
use super::infra::{KeyRecord, SftpRepository};
use crate::audit::domain::{Actor, AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::error_code::{ErrorCode, FieldError};
use crate::iam::domain::{Permission, PermissionSet, Principal};
use crate::iam::infra::UserRepository;
use crate::settings::infra::SettingsRepository;

#[derive(Debug, Clone)]
pub struct SftpState {
    pub pool: PgPool,
}

#[derive(Debug, thiserror::Error)]
pub enum SftpError {
    #[error("invalid field")]
    Invalid(FieldError),
    #[error("conflict: {0:?}")]
    Conflict(ErrorCode),
    #[error("not found: {0:?}")]
    NotFound(ErrorCode),
    #[error("{0}")]
    Internal(String),
}

impl From<sqlx::Error> for SftpError {
    fn from(error: sqlx::Error) -> Self {
        Self::Internal(error.to_string())
    }
}

/// Coordonnées d'un serveur pour un client SFTP.
#[derive(Debug, Clone)]
pub struct Connection {
    /// SFTP ouvert dans les paramètres du projet.
    pub enabled: bool,
    /// Le nœud sert le SFTP et a annoncé sa clé d'hôte (l'hôte peut manquer).
    pub available: bool,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub username: String,
    pub host_key_fingerprint: Option<String>,
    pub node_name: String,
}

pub async fn keys(state: &SftpState, principal: &Principal) -> Result<Vec<KeyRecord>, SftpError> {
    let mut conn = state.pool.acquire().await?;
    Ok(SftpRepository::keys(&mut conn, principal.user_id).await?)
}

/// Ajoute une clé au compte connecté, journalisé avec son empreinte (jamais la clé).
pub async fn add_key(
    state: &SftpState,
    context: &AuditContext,
    principal: &Principal,
    name: &str,
    public_key: &str,
) -> Result<KeyRecord, SftpError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_KEY_NAME_CHARS {
        return Err(SftpError::Invalid(
            FieldError::new("name", ErrorCode::ValueOutOfRange)
                .param("min", 1)
                .param("max", MAX_KEY_NAME_CHARS),
        ));
    }
    let key = domain::parse_public_key(public_key.trim()).map_err(|error| {
        SftpError::Invalid(FieldError::new(
            "public_key",
            match error {
                KeyError::Invalid => ErrorCode::SshKeyInvalid,
                KeyError::Unsupported => ErrorCode::SshKeyUnsupported,
            },
        ))
    })?;
    let id = Uuid::now_v7();
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    if !SftpRepository::insert_key(&mut tx, id, principal.user_id, name, &key).await? {
        return Err(SftpError::Conflict(ErrorCode::SshKeyExists));
    }
    tx.commit(
        AuditEvent::success(AuditAction::OwnSshKeyAdded)
            .target("admin_user", principal.user_id)
            .details(json!({
                "key_id": id,
                "fingerprint": key.fingerprint,
                "algorithm": key.algorithm,
            })),
    )
    .await?;
    let mut conn = state.pool.acquire().await?;
    SftpRepository::keys(&mut conn, principal.user_id)
        .await?
        .into_iter()
        .find(|record| record.id == id)
        .ok_or_else(|| SftpError::Internal("key vanished after insert".to_owned()))
}

pub async fn remove_key(
    state: &SftpState,
    context: &AuditContext,
    principal: &Principal,
    id: Uuid,
) -> Result<(), SftpError> {
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let fingerprint = SftpRepository::delete_key(&mut tx, principal.user_id, id)
        .await?
        .ok_or(SftpError::NotFound(ErrorCode::SshKeyNotFound))?;
    tx.commit(
        AuditEvent::success(AuditAction::OwnSshKeyRemoved)
            .target("admin_user", principal.user_id)
            .details(json!({ "key_id": id, "fingerprint": fingerprint })),
    )
    .await?;
    Ok(())
}

/// Coordonnées SFTP d'un serveur pour le compte connecté ; `None` si le serveur n'existe pas.
pub async fn connection(
    state: &SftpState,
    principal: &Principal,
    server_id: Uuid,
) -> Result<Option<Connection>, SftpError> {
    let mut conn = state.pool.acquire().await?;
    let Some(endpoint) = SftpRepository::endpoint(&mut conn, server_id).await? else {
        return Ok(None);
    };
    let account = SftpRepository::account_handle(&mut conn, principal.user_id)
        .await?
        .ok_or_else(|| SftpError::Internal("account without sftp handle".to_owned()))?;
    let enabled = SettingsRepository::sftp_enabled(&mut conn).await?;
    let port = endpoint.port.and_then(|port| u16::try_from(port).ok());
    Ok(Some(Connection {
        enabled,
        // L'hôte est facultatif : sans DNS ni adresse saisie, le reste est quand même montré.
        available: port.is_some() && endpoint.host_key.is_some(),
        host: endpoint
            .host
            .or_else(|| endpoint.link_address.filter(|address| reachable(address))),
        port,
        username: domain::username(&endpoint.server_handle, &account),
        host_key_fingerprint: endpoint.host_key,
        node_name: endpoint.node_name,
    }))
}

/// Adresse du lien utilisable par un client : ni boucle locale ni adresse non spécifiée (API et
/// nœud sur la même machine, ou relais). Une adresse privée reste montrée (réseau local).
fn reachable(address: &str) -> bool {
    address
        .parse::<std::net::IpAddr>()
        .is_ok_and(|ip| !ip.is_loopback() && !ip.is_unspecified())
}

// --- Requêtes des nœuds --------------------------------------------------------------------

/// Contexte d'audit d'une session SFTP : acteur, client SSH, session comme identifiant de
/// requête.
fn session_context(
    actor: Actor,
    client_ip: &str,
    client_version: &str,
    session_id: Uuid,
) -> AuditContext {
    AuditContext {
        actor,
        ip: Some(truncate(client_ip, 64)),
        user_agent: Some(truncate(client_version, MAX_SSH_CLIENT_VERSION_BYTES)),
        request_id: Some(session_id.to_string()),
        agent: None,
        owner: None,
    }
}

fn truncate(text: &str, max: usize) -> String {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// Résultat d'une vérification des droits, commun à l'authentification et aux baux.
enum Standing {
    Granted {
        write: bool,
    },
    Denied(SftpDenial),
    /// Permission manquante : journalisée en `access.denied`.
    Forbidden(Permission),
}

async fn standing(
    conn: &mut PgConnection,
    node_id: Uuid,
    admin_id: Uuid,
    server_id: Uuid,
) -> Result<Standing, sqlx::Error> {
    if !SettingsRepository::sftp_enabled(conn).await? {
        return Ok(Standing::Denied(SftpDenial::Disabled));
    }
    match SftpRepository::account(conn, admin_id).await? {
        Some(account) if !account.disabled => {}
        _ => return Ok(Standing::Denied(SftpDenial::UnknownAccount)),
    }
    match SftpRepository::server(conn, server_id).await? {
        // Un nœud n'agit que sur les serveurs qui lui sont assignés.
        Some(server) if server.node_id == node_id => {
            if server.suspended {
                return Ok(Standing::Denied(SftpDenial::ServerSuspended));
            }
        }
        _ => return Ok(Standing::Denied(SftpDenial::UnknownServer)),
    }
    let permissions =
        PermissionSet::from_keys(UserRepository::permission_keys(conn, admin_id).await?);
    for required in [Permission::ServersFilesSftp, Permission::ServersFilesRead] {
        if !permissions.contains(required) {
            return Ok(Standing::Forbidden(required));
        }
    }
    Ok(Standing::Granted {
        write: permissions.contains(Permission::ServersFilesWrite),
    })
}

/// Authentifie une connexion SFTP reçue par le nœud `node_id`. Succès (`sftp.session.opened`),
/// échec sur un nom bien formé (`auth.sftp.failed`) et permission manquante (`access.denied`)
/// sont journalisés ; un nom mal formé ne l'est pas (robots qui essaient `root`).
pub async fn authenticate(
    state: &SftpState,
    node_id: Uuid,
    request: SftpAuth,
) -> Result<SftpGrant, SftpError> {
    let denied = |reason| Ok(SftpGrant::Denied { reason });
    let Some((server_handle, account_handle)) = domain::parse_username(&request.username) else {
        return denied(SftpDenial::InvalidUsername);
    };
    let method = match &request.credential {
        SftpCredential::Publickey { .. } => "publickey",
        SftpCredential::Password { .. } => "password",
    };
    let mut conn = state.pool.acquire().await?;
    let account = SftpRepository::account_by_handle(&mut conn, account_handle).await?;
    let server = SftpRepository::server_by_handle(&mut conn, server_handle).await?;
    let actor = account
        .as_ref()
        .map_or_else(Actor::anonymous, |account| Actor::admin(account.id));
    let context = session_context(
        actor,
        &request.client_ip,
        &request.client_version,
        request.session_id,
    );
    let server_id = server.as_ref().map(|server| server.id);
    let failed = |reason: &'static str| {
        let event = AuditEvent::failure(AuditAction::SftpAuthFailed)
            .details(json!({ "reason": reason, "method": method, "node_id": node_id }));
        match server_id {
            Some(id) => event.target("server", id),
            None => event,
        }
    };

    let (Some(account), Some(server)) = (account, server) else {
        record(&state.pool, &context, failed("unknown_account")).await;
        return denied(SftpDenial::UnknownAccount);
    };
    if account.disabled {
        record(&state.pool, &context, failed("unknown_account")).await;
        return denied(SftpDenial::UnknownAccount);
    }
    let credential = match &request.credential {
        SftpCredential::Publickey { algorithm, key } => {
            let key = match domain::from_wire(algorithm, key) {
                Ok(key) => key,
                Err(_) => {
                    record(&state.pool, &context, failed("unknown_key")).await;
                    return denied(SftpDenial::UnknownKey);
                }
            };
            match SftpRepository::key_by_blob(&mut conn, account.id, &key.fingerprint, &key.blob)
                .await?
            {
                Some(id) => (SftpCredentialRef::Key(id), key.fingerprint),
                None => {
                    record(&state.pool, &context, failed("unknown_key")).await;
                    return denied(SftpDenial::UnknownKey);
                }
            }
        }
        // Mot de passe SFTP : tranche 3.
        SftpCredential::Password { .. } => {
            record(&state.pool, &context, failed("bad_password")).await;
            return denied(SftpDenial::BadPassword);
        }
    };

    match standing(&mut conn, node_id, account.id, server.id).await? {
        Standing::Granted { write } => {
            let (credential, fingerprint) = credential;
            if let SftpCredentialRef::Key(id) = credential {
                SftpRepository::touch_key(&mut conn, id).await?;
            }
            drop(conn);
            let tx = AuditedTx::begin(&state.pool, &context).await?;
            tx.commit(
                AuditEvent::success(AuditAction::SftpSessionOpened)
                    .target("server", server.id)
                    .details(json!({
                        "session_id": request.session_id,
                        "node_id": node_id,
                        "method": method,
                        "key_fingerprint": fingerprint,
                        "write": write,
                    })),
            )
            .await?;
            Ok(SftpGrant::Granted {
                server_id: server.id,
                principal: SftpPrincipal::Admin(account.id),
                credential,
                write,
                lease_secs: SFTP_LEASE_SECS,
            })
        }
        Standing::Forbidden(permission) => {
            let event = AuditEvent::denied(AuditAction::AccessDenied)
                .target("server", server.id)
                .details(json!({ "permission": permission.key(), "via": "sftp" }));
            record(&state.pool, &context, event).await;
            denied(SftpDenial::PermissionDenied)
        }
        Standing::Denied(reason) => {
            let code = match reason {
                SftpDenial::Disabled => "sftp_disabled",
                SftpDenial::ServerSuspended => "server_suspended",
                SftpDenial::UnknownServer => "unknown_server",
                _ => "unknown_account",
            };
            record(&state.pool, &context, failed(code)).await;
            denied(reason)
        }
    }
}

/// Événement sans autre écriture ; un échec d'audit est journalisé mais ne change pas le refus.
async fn record(pool: &PgPool, context: &AuditContext, event: AuditEvent) {
    if let Err(error) = crate::audit::application::record(pool, context, event).await {
        tracing::error!(%error, "failed to record sftp audit event");
    }
}

/// Renouvelle les baux : chaque session garde son identifiant (clé encore présente) et ses
/// droits actuels. Rien n'est journalisé ici ; la fermeture qui suit un refus l'est par le nœud.
pub async fn renew(
    state: &SftpState,
    node_id: Uuid,
    request: SftpLease,
) -> Result<SftpLeases, SftpError> {
    let mut conn = state.pool.acquire().await?;
    let mut results = Vec::with_capacity(request.sessions.len());
    for SftpLeaseRequest {
        session_id,
        server_id,
        principal,
        credential,
    } in request.sessions
    {
        let SftpPrincipal::Admin(admin_id) = principal;
        let credential_held = match credential {
            SftpCredentialRef::Key(id) => {
                SftpRepository::key_exists(&mut conn, admin_id, id).await?
            }
            SftpCredentialRef::Password(_) => false,
        };
        let standing = if credential_held {
            standing(&mut conn, node_id, admin_id, server_id).await?
        } else {
            Standing::Denied(SftpDenial::UnknownKey)
        };
        let (granted, write) = match standing {
            Standing::Granted { write } => (true, write),
            Standing::Denied(_) | Standing::Forbidden(_) => (false, false),
        };
        results.push(SftpLeaseResult {
            session_id,
            granted,
            write,
        });
    }
    Ok(SftpLeases { results })
}

/// Enregistre les événements remontés par le nœud, une transaction auditée chacun (accusé et
/// événement ensemble). Un événement déjà reçu est ignoré ; un serveur assigné à un autre nœud
/// fait écarter l'événement (un nœud ne parle que des siens).
pub async fn record_events(
    state: &SftpState,
    node_id: Uuid,
    audit: SftpAudit,
) -> Result<SftpAuditAck, SftpError> {
    let mut recorded = 0u32;
    for event in audit.events.into_iter().take(MAX_SFTP_AUDIT_BATCH) {
        let mut conn = state.pool.acquire().await?;
        let owner = SftpRepository::server(&mut conn, event.server_id).await?;
        drop(conn);
        if owner.is_some_and(|server| server.node_id != node_id) {
            tracing::warn!(server_id = %event.server_id, "sftp event about a server of another node, dropped");
            continue;
        }
        if record_event(state, node_id, event).await? {
            recorded += 1;
        }
    }
    let mut conn = state.pool.acquire().await?;
    SftpRepository::purge_receipts(&mut conn).await?;
    Ok(SftpAuditAck { recorded })
}

/// Écriture faite par SFTP, journalisée comme celle du panel avec `via` et la session.
fn file_event(action: AuditAction, server_id: &Uuid, details: serde_json::Value) -> AuditEvent {
    let mut details = details;
    if let Some(map) = details.as_object_mut() {
        map.insert("via".to_owned(), json!("sftp"));
    }
    AuditEvent::success(action)
        .target("server", server_id)
        .details(details)
}

async fn record_event(
    state: &SftpState,
    node_id: Uuid,
    event: SftpEvent,
) -> Result<bool, SftpError> {
    let SftpPrincipal::Admin(admin_id) = event.principal;
    let context = session_context(
        Actor::admin(admin_id),
        &event.client_ip,
        &event.client_version,
        event.session_id,
    );
    let audit = match event.event {
        SftpEventKind::SessionClosed(summary) => {
            AuditEvent::success(AuditAction::SftpSessionClosed)
                .target("server", event.server_id)
                .details(json!({
                    "session_id": event.session_id,
                    "node_id": node_id,
                    "reason": summary.reason,
                    "duration_s": summary.duration_secs,
                    "bytes_read": summary.bytes_read,
                    "files_read": summary.files_read,
                    "bytes_written": summary.bytes_written,
                    "files_written": summary.files_written,
                    "denied": summary.denied,
                    "node_occurred_at_ms": event.occurred_at_ms,
                }))
        }
        // Mêmes actions que les écritures du panel (ADR-0015), `via` les distingue.
        SftpEventKind::FileWritten { path, size } => file_event(
            AuditAction::ServerFileWritten,
            &event.server_id,
            json!({ "path": truncate(&path, 4096), "size": size }),
        ),
        SftpEventKind::DirectoryCreated { path } => file_event(
            AuditAction::ServerDirectoryCreated,
            &event.server_id,
            json!({ "path": truncate(&path, 4096) }),
        ),
        SftpEventKind::FileRenamed { from, to } => file_event(
            AuditAction::ServerFileRenamed,
            &event.server_id,
            json!({ "from": truncate(&from, 4096), "to": truncate(&to, 4096) }),
        ),
        SftpEventKind::FileDeleted { path } => file_event(
            AuditAction::ServerFileDeleted,
            &event.server_id,
            json!({ "path": truncate(&path, 4096) }),
        ),
        SftpEventKind::ModeChanged { path, mode } => file_event(
            AuditAction::ServerFileModeChanged,
            &event.server_id,
            json!({ "path": truncate(&path, 4096), "mode": format!("{:o}", mode & 0o777) }),
        ),
        SftpEventKind::AccessDenied { permission } => AuditEvent::denied(AuditAction::AccessDenied)
            .target("server", event.server_id)
            .details(json!({
                "permission": truncate(&permission, 64),
                "via": "sftp",
                "session_id": event.session_id,
                "node_id": node_id,
                "node_occurred_at_ms": event.occurred_at_ms,
            })),
    };
    let mut tx = AuditedTx::begin(&state.pool, &context).await?;
    if !SftpRepository::receive(&mut tx, event.event_id).await? {
        return Ok(false);
    }
    tx.commit(audit).await?;
    Ok(true)
}
