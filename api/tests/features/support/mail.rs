//! Client SMTP simulé (ADR-0046) : chaque message est gardé avec le serveur déchiffré qui
//! l'aurait porté, et l'échec se règle à la demande.

use std::sync::Mutex;

use futures_util::future::BoxFuture;
use kuro_api::mail::{Mailer, Message, SmtpTransport};

#[derive(Debug, Default)]
pub struct FakeMailer {
    pub sent: Mutex<Vec<(SmtpTransport, Message)>>,
    pub failing: Mutex<Option<String>>,
}

impl FakeMailer {
    pub fn fail(&self, reason: &str) {
        *self.failing.lock().unwrap() = Some(reason.to_owned());
    }

    pub fn succeed(&self) {
        *self.failing.lock().unwrap() = None;
    }

    pub fn sent(&self) -> Vec<Message> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .map(|(_, message)| message.clone())
            .collect()
    }

    pub fn last_transport(&self) -> Option<SmtpTransport> {
        self.sent
            .lock()
            .unwrap()
            .last()
            .map(|(transport, _)| transport.clone())
    }
}

impl Mailer for FakeMailer {
    fn send(
        &self,
        transport: SmtpTransport,
        message: Message,
    ) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            let failure = self.failing.lock().unwrap().clone();
            if let Some(reason) = failure {
                return Err(reason);
            }
            self.sent.lock().unwrap().push((transport, message));
            Ok(())
        })
    }
}
