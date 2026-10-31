//! Sortie SMTP (ADR-0046) : un message à la fois, TLS obligatoire (STARTTLS ou implicite), par
//! `lettre` en rustls. Derrière un trait pour être simulée dans les tests.

use std::time::Duration;

use futures_util::future::BoxFuture;

const TIMEOUT: Duration = Duration::from_secs(15);

/// Chiffrement de la connexion SMTP. `None` n'est accepté qu'avec `mail.allow_insecure` (boîte
/// de dev), jamais en production.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    /// Connexion en clair puis `STARTTLS` (port 587 en général).
    StartTls,
    /// TLS dès la connexion (port 465 en général).
    Tls,
    /// En clair, sans chiffrement : dev seulement.
    None,
}

impl Security {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::StartTls => "starttls",
            Self::Tls => "tls",
            Self::None => "none",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "starttls" => Some(Self::StartTls),
            "tls" => Some(Self::Tls),
            "none" => Some(Self::None),
            _ => None,
        }
    }
}

/// Serveur SMTP déchiffré, le temps d'un envoi.
#[derive(Clone)]
pub struct SmtpTransport {
    pub host: String,
    pub port: u16,
    pub security: Security,
    pub username: Option<String>,
    pub password: Option<String>,
    pub from_address: String,
    pub from_name: String,
}

impl std::fmt::Debug for SmtpTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmtpTransport")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("security", &self.security)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("from_address", &self.from_address)
            .finish_non_exhaustive()
    }
}

/// Image embarquée dans le HTML (`<img src="cid:…">`), par exemple le logo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineImage {
    pub cid: &'static str,
    pub content_type: &'static str,
    pub bytes: &'static [u8],
}

/// Message prêt à partir : destinataires, sujet, corps texte et HTML, images inline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub to: Vec<String>,
    pub subject: String,
    pub text: String,
    pub html: Option<String>,
    pub inline: Vec<InlineImage>,
}

impl Message {
    /// Message rendu par le gabarit du projet, logo compris.
    pub fn from_template(to: Vec<String>, email: &super::template::Email) -> Self {
        Self {
            to,
            subject: email.subject(),
            text: email.text(),
            html: Some(email.html()),
            inline: vec![InlineImage {
                cid: super::template::LOGO_CID,
                content_type: "image/png",
                bytes: super::template::LOGO_PNG,
            }],
        }
    }
}

/// Envoi d'un message ; `Err` porte une cause courte (réponse SMTP, délai), sans secret.
pub trait Mailer: Send + Sync + std::fmt::Debug {
    fn send(&self, transport: SmtpTransport, message: Message)
    -> BoxFuture<'_, Result<(), String>>;
}

/// Client SMTP réel (`lettre`, rustls, racines webpki).
#[derive(Debug, Clone, Default)]
pub struct LettreMailer;

impl Mailer for LettreMailer {
    fn send(
        &self,
        transport: SmtpTransport,
        message: Message,
    ) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            use lettre::message::header::ContentType;
            use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart};
            use lettre::transport::smtp::authentication::Credentials;
            use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

            let from = Mailbox::new(
                Some(transport.from_name.clone()),
                transport
                    .from_address
                    .parse()
                    .map_err(|_| "invalid_from_address".to_owned())?,
            );
            let mut builder = lettre::Message::builder()
                .from(from)
                .subject(message.subject.clone());
            for to in &message.to {
                builder = builder.to(Mailbox::new(
                    None,
                    to.parse().map_err(|_| "invalid_recipient".to_owned())?,
                ));
            }
            let email = match message.html {
                Some(html) if message.inline.is_empty() => builder.multipart(
                    MultiPart::alternative_plain_html(message.text.clone(), html),
                ),
                // HTML avec images inline : alternative(texte, related(html, images)).
                Some(html) => {
                    let mut related = MultiPart::related().singlepart(SinglePart::html(html));
                    for image in &message.inline {
                        let content_type = ContentType::parse(image.content_type)
                            .map_err(|_| "invalid_inline_content_type".to_owned())?;
                        related = related.singlepart(
                            Attachment::new_inline(image.cid.to_owned())
                                .body(image.bytes.to_vec(), content_type),
                        );
                    }
                    builder.multipart(
                        MultiPart::alternative()
                            .singlepart(SinglePart::plain(message.text.clone()))
                            .multipart(related),
                    )
                }
                None => builder.body(message.text.clone()),
            }
            .map_err(|error| format!("message: {error}"))?;

            let mut smtp = match transport.security {
                Security::StartTls => {
                    AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&transport.host)
                }
                Security::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&transport.host),
                // Sans chiffrement : seulement si l'API l'a permis (`mail.allow_insecure`).
                Security::None => Ok(AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(
                    &transport.host,
                )),
            }
            .map_err(|error| format!("smtp: {error}"))?
            .port(transport.port)
            .timeout(Some(TIMEOUT));
            if let (Some(username), Some(password)) = (transport.username, transport.password) {
                smtp = smtp.credentials(Credentials::new(username, password));
            }
            smtp.build()
                .send(email)
                .await
                .map(|_| ())
                .map_err(|error| format!("smtp: {error}"))
        })
    }
}
