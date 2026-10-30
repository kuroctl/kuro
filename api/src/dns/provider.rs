//! Fournisseur DNS (ADR-0026) : ce que Kuro attend d'une zone, quel que soit l'hébergeur.

use futures_util::future::BoxFuture;

use super::domain::Desired;

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// Jeton refusé ou sans droit sur la zone.
    #[error("dns provider refused the token")]
    Unauthorized,
    #[error("dns zone not found")]
    ZoneNotFound,
    /// Panne, délai dépassé, réponse inattendue.
    #[error("dns provider unavailable: {0}")]
    Unavailable(String),
    /// Enregistrement refusé (conflit avec un enregistrement existant…).
    #[error("dns provider refused the record: {0}")]
    Rejected(String),
}

/// Accès à une zone chez le fournisseur.
#[derive(Clone)]
pub struct ZoneAccess {
    /// Identifiant de la zone chez le fournisseur.
    pub zone: String,
    pub token: String,
    pub ttl: u32,
}

impl std::fmt::Debug for ZoneAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZoneAccess")
            .field("zone", &self.zone)
            .field("ttl", &self.ttl)
            .finish_non_exhaustive()
    }
}

/// Enregistrement existant chez le fournisseur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    pub id: String,
    pub record: Desired,
}

pub trait DnsProvider: Send + Sync + std::fmt::Debug {
    /// Zone qui contient `domain` (`play.kuroctl.com` est dans la zone `kuroctl.com`) : son
    /// identifiant chez le fournisseur.
    fn find_zone<'a>(
        &'a self,
        token: &'a str,
        domain: &'a str,
    ) -> BoxFuture<'a, Result<String, ProviderError>>;

    /// Enregistrements de la zone.
    fn list<'a>(
        &'a self,
        access: &'a ZoneAccess,
    ) -> BoxFuture<'a, Result<Vec<Remote>, ProviderError>>;

    /// Crée l'enregistrement ; rend son identifiant.
    fn create<'a>(
        &'a self,
        access: &'a ZoneAccess,
        record: &'a Desired,
    ) -> BoxFuture<'a, Result<String, ProviderError>>;

    fn update<'a>(
        &'a self,
        access: &'a ZoneAccess,
        id: &'a str,
        record: &'a Desired,
    ) -> BoxFuture<'a, Result<(), ProviderError>>;

    /// Supprime l'enregistrement ; déjà absent n'est pas une erreur.
    fn delete<'a>(
        &'a self,
        access: &'a ZoneAccess,
        id: &'a str,
    ) -> BoxFuture<'a, Result<(), ProviderError>>;
}
