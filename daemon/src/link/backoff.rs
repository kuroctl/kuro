//! Délai de reconnexion : doublé à chaque échec, plafonné, avec une part aléatoire
//! pour que les nœuds ne se reconnectent pas tous en même temps après une coupure de l'API.

use std::time::Duration;

const INITIAL: Duration = Duration::from_secs(1);
const MAX: Duration = Duration::from_secs(60);

#[derive(Debug)]
pub(super) struct Backoff {
    next: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self { next: INITIAL }
    }
}

impl Backoff {
    /// Délai à attendre, entre 50 % et 100 % du palier courant.
    pub(super) fn next_delay(&mut self) -> Duration {
        let base = self.next;
        self.next = (self.next * 2).min(MAX);
        base.mul_f64(0.5 + fastrand::f64() * 0.5)
    }

    /// À appeler une fois le lien établi.
    pub(super) fn reset(&mut self) {
        self.next = INITIAL;
    }
}
