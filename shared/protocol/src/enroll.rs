//! Échange REST d'enrôlement : `POST /api/enroll` sur le listener daemon.
//!
//! Le daemon envoie sa CSR et le token à usage unique ; l'API répond avec le certificat
//! signé et la CA. La clé privée ne quitte jamais l'hôte.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Chemin HTTP de l'enrôlement.
pub const ENROLL_PATH: &str = "/api/enroll";

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EnrollRequest {
    /// Token à usage unique affiché par `kuro-api node add`.
    pub token: String,
    /// Demande de certificat (PKCS#10, PEM) générée sur l'hôte.
    pub csr_pem: String,
}

/// Le token est un secret : jamais affiché, même en debug.
impl std::fmt::Debug for EnrollRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnrollRequest")
            .field("token", &"<redacted>")
            .field("csr_pem", &self.csr_pem)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EnrollResponse {
    pub node_id: Uuid,
    pub node_name: String,
    /// Certificat client du nœud (PEM), signé par la CA.
    pub cert_pem: String,
    /// Certificat de la CA (PEM), à utiliser comme seule racine de confiance.
    pub ca_pem: String,
}
