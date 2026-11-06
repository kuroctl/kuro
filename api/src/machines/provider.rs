//! Frontière avec un hébergeur (ADR-0065) : le seul contrat qu'un fournisseur implémente. Types
//! neutres, aucun nom propre à un hébergeur ; tout le métier est dans `service`.

use std::collections::BTreeMap;

use futures_util::future::BoxFuture;

/// Type de machine proposé dans un emplacement.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
pub struct Offer {
    /// Nom chez le fournisseur (`cx22`).
    pub name: String,
    pub vcpus: u32,
    pub memory_mb: u32,
    pub disk_gb: u32,
    /// Prix horaire HT, en millièmes de centime (1/100 000 d'euro).
    pub hourly_cost: i64,
    /// Prix mensuel plafonné HT, en centimes.
    pub monthly_cost: i64,
}

/// Emplacement d'un fournisseur (centre de données), pour y relier une région.
#[derive(Debug, Clone, PartialEq, serde::Serialize, utoipa::ToSchema)]
pub struct Location {
    /// Code chez le fournisseur (`fsn1`).
    pub name: String,
    pub city: String,
    /// Code pays ISO (`DE`).
    pub country: String,
    pub latitude: f64,
    pub longitude: f64,
}

/// Machine à créer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineSpec {
    pub name: String,
    pub offer: String,
    pub location: String,
    /// Document cloud-init lancé au premier démarrage.
    pub user_data: String,
    pub labels: BTreeMap<String, String>,
    /// Snapshot de départ (machine recréée) ; absent : image de base du fournisseur.
    pub snapshot: Option<String>,
}

/// Avancement d'un snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotState {
    Pending,
    Ready,
    /// Absent chez le fournisseur.
    Missing,
}

/// Machine créée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Machine {
    /// Identifiant chez le fournisseur.
    pub reference: String,
    pub ipv4: Option<String>,
}

/// Machine trouvée chez le fournisseur par son étiquette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub reference: String,
    /// Valeur de l'étiquette (identifiant de la machine chez Kuro).
    pub label: String,
    pub ipv4: Option<String>,
    pub created_at: time::OffsetDateTime,
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// Refus du fournisseur (quota, type indisponible…) : code et message pour le back-office.
    #[error("provider refused: {code} {message}")]
    Refused { code: String, message: String },
    #[error("provider unreachable: {0}")]
    Transport(String),
}

pub trait Provider: Send + Sync + std::fmt::Debug {
    /// Emplacements où l'on peut louer.
    fn locations(&self) -> BoxFuture<'_, Result<Vec<Location>, ProviderError>>;
    /// Types de machine x86 disponibles dans `location`.
    fn offers<'a>(&'a self, location: &'a str) -> BoxFuture<'a, Result<Vec<Offer>, ProviderError>>;
    fn create(&self, spec: MachineSpec) -> BoxFuture<'_, Result<Machine, ProviderError>>;
    /// Toutes les machines qui portent l'étiquette `label` (quelle qu'en soit la valeur) et
    /// l'étiquette `scope` (clé, valeur exacte) : celles de cette instance de Kuro.
    fn list<'a>(
        &'a self,
        label: &'a str,
        scope: (&'a str, &'a str),
    ) -> BoxFuture<'a, Result<Vec<Listed>, ProviderError>>;
    /// Supprime une machine ; une machine déjà absente n'est pas une erreur.
    fn delete<'a>(&'a self, reference: &'a str) -> BoxFuture<'a, Result<(), ProviderError>>;
    /// Demande un snapshot de la machine (étiqueté comme elle) ; sa référence. Il se termine plus
    /// tard : la machine ne se supprime qu'une fois le snapshot prêt.
    fn snapshot<'a>(
        &'a self,
        reference: &'a str,
        labels: &'a BTreeMap<String, String>,
    ) -> BoxFuture<'a, Result<String, ProviderError>>;
    fn snapshot_state<'a>(
        &'a self,
        snapshot: &'a str,
    ) -> BoxFuture<'a, Result<SnapshotState, ProviderError>>;
    /// Supprime un snapshot ; déjà absent n'est pas une erreur.
    fn delete_snapshot<'a>(&'a self, snapshot: &'a str)
    -> BoxFuture<'a, Result<(), ProviderError>>;
}
