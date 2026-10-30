use std::ops::{Deref, DerefMut};

use sqlx::{PgConnection, PgPool, Postgres, Transaction};

use super::AuditRepository;
use crate::audit::domain::{Actor, AuditContext, AuditEvent};

/// Transaction d'écriture métier qui ne peut être validée **qu'avec** son événement d'audit,
/// inséré dans la même transaction. Abandonnée (drop) sans `commit`, elle est annulée et ne
/// laisse ni modification ni trace.
///
/// Les repositories la reçoivent comme une connexion (`&mut tx`).
pub struct AuditedTx {
    tx: Transaction<'static, Postgres>,
    context: AuditContext,
    recorded: Vec<(AuditContext, AuditEvent)>,
}

impl AuditedTx {
    pub async fn begin(pool: &PgPool, context: &AuditContext) -> Result<Self, sqlx::Error> {
        Ok(Self {
            tx: pool.begin().await?,
            context: context.clone(),
            recorded: Vec::new(),
        })
    }

    pub fn context(&self) -> &AuditContext {
        &self.context
    }

    /// L'acteur est identifié en cours de route (connexion réussie, jeton reconnu).
    pub fn set_actor(&mut self, actor: Actor) {
        self.context.actor = actor;
    }

    /// Événement supplémentaire, avant celui qui valide la transaction.
    pub async fn record(&mut self, event: AuditEvent) -> Result<(), sqlx::Error> {
        AuditRepository::append(&mut self.tx, &self.context, &event).await?;
        self.recorded.push((self.context.clone(), event));
        Ok(())
    }

    pub async fn commit(mut self, event: AuditEvent) -> Result<(), sqlx::Error> {
        AuditRepository::append(&mut self.tx, &self.context, &event).await?;
        self.tx.commit().await?;
        for (context, event) in &self.recorded {
            emit(context, event);
        }
        emit(&self.context, &event);
        Ok(())
    }
}

impl Deref for AuditedTx {
    type Target = PgConnection;

    fn deref(&self) -> &PgConnection {
        &self.tx
    }
}

impl DerefMut for AuditedTx {
    fn deref_mut(&mut self) -> &mut PgConnection {
        &mut self.tx
    }
}

/// Copie de l'événement dans les logs (cible `audit`), à expédier hors de l'hôte (SIEM, WORM).
fn emit(context: &AuditContext, event: &AuditEvent) {
    tracing::info!(
        target: "audit",
        action = event.action.code(),
        outcome = event.outcome.as_str(),
        actor_kind = context.actor.kind.as_str(),
        actor_id = context.actor.id.map(|id| id.to_string()),
        target_kind = event.target.as_ref().map(|target| target.kind),
        target_id = event.target.as_ref().map(|target| target.id.as_str()),
        ip = context.ip.as_deref(),
        request_id = context.request_id.as_deref(),
        details = %event.details,
        "audit event"
    );
}
