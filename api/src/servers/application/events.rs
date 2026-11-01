//! Diffusion des événements du cycle de vie aux extensions (ADR-0051 § 6) : après la
//! validation de la transaction, dans une tâche à part, chaque extension à son tour ; une erreur
//! ou un dépassement de délai est journalisé et n'atteint jamais l'action de Kuro.

use std::sync::Arc;
use std::time::Duration;

use crate::extension::{MountedExtension, ServerHooks};
use crate::servers::domain::ServerEvent;

/// Délai accordé à une extension pour un événement.
const HOOK_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Default)]
pub struct ServerEventSink(Arc<Vec<(&'static str, Arc<dyn ServerHooks>)>>);

impl ServerEventSink {
    /// Celles des extensions préparées qui écoutent les serveurs.
    pub fn of(extensions: &[MountedExtension]) -> Self {
        Self(Arc::new(
            extensions
                .iter()
                .filter_map(|e| e.hooks.clone().map(|hooks| (e.name, hooks)))
                .collect(),
        ))
    }

    pub fn emit(&self, event: ServerEvent) {
        if self.0.is_empty() {
            return;
        }
        let listeners = self.0.clone();
        tokio::spawn(async move {
            for (extension, hooks) in listeners.iter() {
                let outcome =
                    tokio::time::timeout(HOOK_TIMEOUT, hooks.on_server_event(&event)).await;
                let error = match outcome {
                    Ok(Ok(())) => continue,
                    Ok(Err(error)) => error.to_string(),
                    Err(_) => "timed out".to_owned(),
                };
                tracing::warn!(
                    extension,
                    event = event.kind(),
                    server_id = %event.server_id(),
                    %error,
                    "extension failed to handle a server event"
                );
            }
        });
    }
}

impl std::fmt::Debug for ServerEventSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.0.iter().map(|(name, _)| name))
            .finish()
    }
}
