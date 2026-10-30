//! Disponibilité de l'instance : ce que la sonde `/api/ready` vérifie en plus de la vivacité.
//!
//! La base est interrogée à chaque appel ; le listener daemon tient son état à jour lui-même.
//! Un répartiteur de charge retire l'instance du pool dès qu'une vérification échoue.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sqlx::PgPool;

/// Délai accordé à la base, ouverture d'une connexion comprise : une sonde ne doit jamais
/// traîner, mais un pool vide après une période creuse doit avoir le temps de se reconnecter.
const DATABASE_TIMEOUT: Duration = Duration::from_secs(5);

/// État partagé entre le listener daemon et la route `/api/ready`.
#[derive(Debug, Clone, Default)]
pub struct Readiness {
    listener: Arc<AtomicBool>,
}

impl Readiness {
    /// Le listener accepte les connexions des daemons.
    pub fn listener_up(&self) {
        self.listener.store(true, Ordering::Release);
    }

    /// Le listener s'est arrêté (arrêt du service).
    pub fn listener_down(&self) {
        self.listener.store(false, Ordering::Release);
    }

    pub fn listener_is_up(&self) -> bool {
        self.listener.load(Ordering::Acquire)
    }

    /// Vérifications complètes : base joignable (une requête, cinq secondes au plus) et
    /// listener en marche.
    pub async fn check(&self, pool: &PgPool) -> Report {
        let database =
            match tokio::time::timeout(DATABASE_TIMEOUT, sqlx::query("SELECT 1").execute(pool))
                .await
            {
                Ok(Ok(_)) => true,
                Ok(Err(error)) => {
                    tracing::warn!(%error, "readiness: database check failed");
                    false
                }
                Err(_) => {
                    tracing::warn!("readiness: database check timed out");
                    false
                }
            };
        Report {
            database,
            daemon_listener: self.listener_is_up(),
        }
    }
}

/// Résultat des vérifications, une par dépendance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    pub database: bool,
    pub daemon_listener: bool,
}

impl Report {
    pub fn ready(self) -> bool {
        self.database && self.daemon_listener
    }
}
