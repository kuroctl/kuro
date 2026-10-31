//! Envoi d'e-mails (ADR-0046) : réglages SMTP du projet, chiffrés, un envoi de test, et les
//! messages des notifications et des invitations. `sender` est le client SMTP (simulé dans les
//! tests), `application` les cas d'usage, `http` les routes des paramètres.

pub mod application;
pub mod http;
pub mod sender;
pub mod template;

pub use application::{MailError, MailInput, MailSettingsView, MailState};
pub use sender::{InlineImage, LettreMailer, Mailer, Message, Security, SmtpTransport};
pub use template::Email;
