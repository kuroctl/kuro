//! Webhooks simulés (ADR-0044) : chaque envoi est gardé, et la réponse se règle par URL.

use std::collections::HashMap;
use std::sync::Mutex;

use futures_util::future::BoxFuture;
use kuro_api::notifications::sender::{Outgoing, WebhookSender};

#[derive(Debug, Default)]
pub struct FakeWebhooks {
    /// Envois reçus, dans l'ordre, avec le réglage « adresses privées acceptées ».
    pub sent: Mutex<Vec<(Outgoing, bool)>>,
    /// URL → cause d'échec ; les autres URL répondent 204.
    pub failing: Mutex<HashMap<String, String>>,
}

impl FakeWebhooks {
    pub fn fail(&self, url: &str, reason: &str) {
        self.failing
            .lock()
            .unwrap()
            .insert(url.to_owned(), reason.to_owned());
    }

    pub fn succeed(&self, url: &str) {
        self.failing.lock().unwrap().remove(url);
    }

    pub fn sent(&self) -> Vec<Outgoing> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .map(|(outgoing, _)| outgoing.clone())
            .collect()
    }
}

impl WebhookSender for FakeWebhooks {
    fn send(&self, outgoing: Outgoing, allow_private: bool) -> BoxFuture<'_, Result<u16, String>> {
        Box::pin(async move {
            let failure = self.failing.lock().unwrap().get(&outgoing.url).cloned();
            self.sent.lock().unwrap().push((outgoing, allow_private));
            match failure {
                Some(reason) => Err(reason),
                None => Ok(204),
            }
        })
    }
}
