//! Déploiement d'essai d'un brouillon du Studio (ADR-0011).
//!
//! Le brouillon est figé en une version d'essai cachée (`1.0.1-trial.<n>`, hors du catalogue),
//! déployée sur un nœud de test (`nodes.trials`) en serveur ordinaire, créé par le même cas
//! d'usage que les autres. Un essai dure [`TRIAL_DURATION`] ; un seul par brouillon (en
//! relancer un remplace le précédent). Le passage périodique ([`sweep`]) supprime les essais
//! expirés, puis les versions d'essai qu'aucun serveur n'utilise plus.

use serde_json::{Map, json};
use time::OffsetDateTime;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::error_code::{ErrorCode, FieldError};
use crate::nodes::infra::NodeRepository;
use crate::prefabs::application::{self as prefabs, ImportAs};
use crate::servers::application::{
    self as servers, NewServer, ServerView, ServersError, ServersState,
};
use crate::servers::infra::ServerRepository;
use crate::studio::application as studio;

/// Durée d'un essai.
pub const TRIAL_DURATION: time::Duration = time::Duration::hours(2);

/// Intervalle du passage de nettoyage.
const SWEEP_EVERY: std::time::Duration = std::time::Duration::from_secs(60);

/// Pourquoi un essai s'arrête (journalisé).
#[derive(Debug, Clone, Copy)]
pub enum EndReason {
    Expired,
    Replaced,
    Published,
    DraftDeleted,
}

impl EndReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Expired => "expired",
            Self::Replaced => "replaced",
            Self::Published => "published",
            Self::DraftDeleted => "draft_deleted",
        }
    }
}

/// `1.0.1` → `1.0.1-trial.<n>` : jamais une version du catalogue, unique par essai.
fn trial_version(base: &semver::Version, now: OffsetDateTime) -> semver::Version {
    let mut version = semver::Version::new(base.major, base.minor, base.patch);
    let stamp = now.unix_timestamp_nanos() / 1_000_000;
    version.pre =
        semver::Prerelease::new(&format!("trial.{stamp}")).unwrap_or(semver::Prerelease::EMPTY);
    version
}

/// Lance l'essai du brouillon sur un nœud de test ; remplace l'essai en cours s'il y en a un.
pub async fn start(
    state: &ServersState,
    context: &AuditContext,
    draft: Uuid,
    node_id: Uuid,
    name: &str,
    now: OffsetDateTime,
) -> Result<ServerView, ServersError> {
    {
        let mut conn = state.pool.acquire().await?;
        let node = NodeRepository::get(&mut conn, node_id)
            .await?
            .ok_or_else(|| {
                ServersError::Invalid(FieldError::new("node_id", ErrorCode::NotFound))
            })?;
        if !node.trials {
            return Err(ServersError::Conflict(ErrorCode::NodeNotForTrials));
        }
    }
    let loaded = studio::draft_package(&state.pool, draft).await?;
    let version = loaded
        .as_ref()
        .map(|(package, _)| trial_version(&package.manifest.version, now))
        .unwrap_or_else(|_| semver::Version::new(0, 0, 0));
    end(state, context, draft, EndReason::Replaced).await?;

    let imported = prefabs::import_loaded(
        &state.pool,
        context,
        loaded,
        json!({ "draft": draft }),
        ImportAs::Trial { draft, version },
    )
    .await?;
    let created = servers::create(
        state,
        context,
        NewServer {
            name: name.to_owned(),
            node_id,
            prefab_id: imported.id.clone(),
            prefab_version: Some(imported.version.clone()),
            memory_mb: None,
            cpu_percent: None,
            disk_mb: None,
            settings: Map::new(),
            allocations: std::collections::BTreeMap::new(),
            start_when_installed: true,
            backup_limit: Some(0),
            owner_ref: None,
        },
    )
    .await;
    let view = match created {
        Ok(view) => view,
        Err(error) => {
            // Rien à essayer : la version d'essai ne sert à aucun serveur.
            if let Err(cleanup) = prefabs::delete_unused_trials(&state.pool, context).await {
                tracing::warn!(error = %cleanup, "failed to remove an unused trial version");
            }
            return Err(error);
        }
    };

    let expires_at = now + TRIAL_DURATION;
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    ServerRepository::mark_trial(&mut tx, view.server.id, draft, expires_at).await?;
    tx.commit(
        AuditEvent::success(AuditAction::PrefabDraftTrialStarted)
            .target("server", view.server.id)
            .details(json!({
                "draft": draft,
                "node_id": node_id,
                "prefab": imported.id,
                "version": imported.version,
                "expires_at": expires_at.unix_timestamp(),
            })),
    )
    .await?;
    servers::get(state, view.server.id).await
}

/// Arrête les essais en cours du brouillon (suppression demandée au nœud). Un nœud injoignable
/// laisse l'essai à la passe suivante, qui le supprime à son expiration.
pub async fn end(
    state: &ServersState,
    context: &AuditContext,
    draft: Uuid,
    reason: EndReason,
) -> Result<usize, ServersError> {
    let ids = running(state, draft).await?;
    Ok(stop_all(state, context, &ids, draft, reason).await)
}

/// Serveurs d'essai en cours du brouillon.
pub async fn running(state: &ServersState, draft: Uuid) -> Result<Vec<Uuid>, ServersError> {
    let mut conn = state.pool.acquire().await?;
    Ok(ServerRepository::trials_of(&mut conn, draft).await?)
}

/// Arrête ces serveurs d'essai ; rendu : combien l'ont été.
pub async fn stop_all(
    state: &ServersState,
    context: &AuditContext,
    servers: &[Uuid],
    draft: Uuid,
    reason: EndReason,
) -> usize {
    let mut ended = 0;
    for &id in servers {
        if stop(state, context, id, Some(draft), reason).await {
            ended += 1;
        }
    }
    ended
}

async fn stop(
    state: &ServersState,
    context: &AuditContext,
    server: Uuid,
    draft: Option<Uuid>,
    reason: EndReason,
) -> bool {
    match servers::delete(state, context, server).await {
        Ok(()) => {
            let event = AuditEvent::success(AuditAction::PrefabDraftTrialEnded)
                .target("server", server)
                .details(json!({ "draft": draft, "reason": reason.as_str() }));
            if let Err(error) = crate::audit::application::record(&state.pool, context, event).await
            {
                tracing::error!(%error, "failed to record the end of a trial");
            }
            true
        }
        Err(error) => {
            tracing::warn!(%error, %server, "trial server not removed yet");
            false
        }
    }
}

/// Supprime les essais expirés, puis les versions d'essai inutilisées.
pub async fn sweep(state: &ServersState, now: OffsetDateTime) -> Result<(), ServersError> {
    let context = AuditContext::system();
    let expired = {
        let mut conn = state.pool.acquire().await?;
        let ids = ServerRepository::expired_trials(&mut conn, now).await?;
        let mut expired = Vec::with_capacity(ids.len());
        for id in ids {
            let draft = ServerRepository::get(&mut conn, id)
                .await?
                .and_then(|server| server.trial_draft_id);
            expired.push((id, draft));
        }
        expired
    };
    for (id, draft) in expired {
        stop(state, &context, id, draft, EndReason::Expired).await;
    }
    prefabs::delete_unused_trials(&state.pool, &context).await?;
    Ok(())
}

/// Passe de nettoyage, chaque minute, jusqu'à l'arrêt de l'API.
pub async fn sweep_forever(state: ServersState, shutdown: CancellationToken) {
    loop {
        if let Err(error) = sweep(&state, OffsetDateTime::now_utc()).await {
            tracing::error!(%error, "trial sweep failed");
        }
        tokio::select! {
            () = shutdown.cancelled() => return,
            () = tokio::time::sleep(SWEEP_EVERY) => {}
        }
    }
}
