//! Échange d'un token valide et d'une CSR contre le certificat client du nœud.

use serde_json::json;
use sqlx::PgPool;

use crate::audit::domain::{Actor, ActorKind, AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::enrollment::domain::TokenHash;
use crate::enrollment::infra::EnrollmentRepository;
use crate::nodes::domain::{CertFingerprint, Node};
use crate::nodes::infra::NodeRepository;
use crate::pki::domain::{CertificateAuthority, PkiError};

#[derive(Debug, thiserror::Error)]
pub enum EnrollError {
    /// Inconnu, expiré, déjà utilisé ou nœud déjà enrôlé : volontairement indiscernables.
    #[error("invalid enrollment token")]
    InvalidToken,
    /// La CSR est refusée avant de consommer le token.
    #[error(transparent)]
    InvalidCsr(PkiError),
    #[error(transparent)]
    Pki(PkiError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

#[derive(Debug)]
pub struct Enrollment {
    pub node: Node,
    pub cert_pem: String,
    pub ca_pem: String,
}

/// Le token n'est consommé que si tout réussit (une seule transaction). Les échecs sont
/// journalisés à part, la transaction étant annulée.
pub async fn enroll(
    pool: &PgPool,
    context: &AuditContext,
    ca: &CertificateAuthority,
    token: &str,
    csr_pem: &str,
    validity: time::Duration,
) -> Result<Enrollment, EnrollError> {
    let result = try_enroll(pool, context, ca, token, csr_pem, validity).await;
    let reason = match &result {
        Err(EnrollError::InvalidToken) => "invalid_token",
        Err(EnrollError::InvalidCsr(_)) => "invalid_csr",
        _ => return result,
    };
    let event = AuditEvent::failure(AuditAction::NodeEnrolled).details(json!({ "reason": reason }));
    crate::audit::application::record(pool, context, event).await?;
    result
}

async fn try_enroll(
    pool: &PgPool,
    context: &AuditContext,
    ca: &CertificateAuthority,
    token: &str,
    csr_pem: &str,
    validity: time::Duration,
) -> Result<Enrollment, EnrollError> {
    let mut tx = AuditedTx::begin(pool, context).await?;
    let claimed = EnrollmentRepository::claim(&mut tx, &TokenHash::of(token))
        .await?
        .ok_or(EnrollError::InvalidToken)?;
    let node = NodeRepository::lock_pending(&mut tx, claimed.node_id)
        .await?
        .ok_or(EnrollError::InvalidToken)?;

    let signed = ca
        .sign_node_csr(csr_pem, &node.name, validity)
        .map_err(|error| match error {
            PkiError::InvalidCsr(_) => EnrollError::InvalidCsr(error),
            other => EnrollError::Pki(other),
        })?;
    let fingerprint = CertFingerprint::from_der(&signed.der);

    EnrollmentRepository::mark_used(&mut tx, claimed.id).await?;
    NodeRepository::complete_enrollment(&mut tx, node.id, &fingerprint).await?;
    tx.set_actor(Actor {
        kind: ActorKind::Node,
        id: Some(node.id.as_uuid()),
    });
    tx.commit(
        AuditEvent::success(AuditAction::NodeEnrolled)
            .target("node", node.id)
            .details(json!({ "name": node.name, "certificate_fingerprint": fingerprint.as_str() })),
    )
    .await?;

    Ok(Enrollment {
        node,
        cert_pem: signed.cert_pem,
        ca_pem: ca.cert_pem().to_owned(),
    })
}
