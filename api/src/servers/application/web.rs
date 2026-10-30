//! Sites web (ADR-0038) : identifiant et adresse d'un serveur dont le prefab a un port web.
//!
//! L'adresse est `https://<identifiant>.<domaine des sites>` ; le domaine des sites vient de la
//! zone DNS (ADR-0026). Sans lui, un prefab web n'est pas déployable : l'API le refuse avant de
//! créer quoi que ce soit, plutôt que de laisser un serveur sans adresse.

use kuro_runtime::tls;
use sqlx::PgPool;

use super::ServersError;
use crate::dns::domain::{WEB_ID_LEN, web_id_from};
use crate::dns::infra::DnsRepository;
use crate::error_code::ErrorCode;
use crate::servers::infra::ServerRecord;

/// Domaine des sites de la zone DNS, si elle est configurée, activée et en a un.
pub async fn sites_domain(pool: &PgPool) -> Result<Option<String>, ServersError> {
    let mut conn = pool.acquire().await?;
    Ok(DnsRepository::zone(&mut conn)
        .await?
        .filter(|zone| zone.enabled)
        .and_then(|zone| zone.sites_domain))
}

/// Nouvel identifiant de site, tiré du générateur cryptographique de l'hôte ; refusé sans domaine
/// des sites (`sites_domain_not_configured`).
pub async fn new_web_id(pool: &PgPool) -> Result<String, ServersError> {
    if sites_domain(pool).await?.is_none() {
        return Err(ServersError::Unprocessable(
            ErrorCode::SitesDomainNotConfigured,
        ));
    }
    let random =
        tls::random_bytes::<WEB_ID_LEN>().map_err(|e| ServersError::Internal(e.to_string()))?;
    Ok(web_id_from(&random))
}

/// `https://<identifiant>.<domaine des sites>` d'un serveur web ; absente pour un serveur de jeu,
/// ou si le domaine des sites a été retiré depuis (le prefab ne se résout alors plus).
pub async fn web_url(pool: &PgPool, server: &ServerRecord) -> Result<Option<String>, ServersError> {
    let Some(web_id) = &server.web_id else {
        return Ok(None);
    };
    Ok(sites_domain(pool)
        .await?
        .map(|sites| format!("https://{web_id}.{sites}")))
}
