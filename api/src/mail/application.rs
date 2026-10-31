//! Réglages SMTP du projet (ADR-0046) : une ligne chiffrée, un envoi de test, et l'envoi des
//! messages pour les notifications et les invitations. Le mot de passe n'est jamais rendu ni
//! journalisé.

use std::sync::Arc;

use serde_json::json;
use sqlx::{PgConnection, PgPool};
use time::OffsetDateTime;

use super::sender::{Mailer, Message, Security, SmtpTransport};
use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::auth::secret_box;
use crate::error_code::{ErrorCode, FieldError};
use crate::iam::domain::Email;
use crate::settings::infra::SettingsRepository;

/// Nom d'expéditeur quand ni le réglage ni le nom du projet ne sont donnés.
const DEFAULT_BRAND: &str = "Kuro";

#[derive(Debug, Clone)]
pub struct MailState {
    pub pool: PgPool,
    /// Clé de chiffrement du mot de passe SMTP (celle des secrets des serveurs).
    pub key: [u8; 32],
    pub mailer: Arc<dyn Mailer>,
    /// URL du panel, pour les liens des messages ; absente : liens omis.
    pub panel_url: Option<String>,
    /// SMTP sans chiffrement accepté (`mail.allow_insecure`) : boîte de dev seulement.
    pub allow_insecure: bool,
}

/// Réglages tels que rendus : jamais le mot de passe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailSettingsView {
    pub host: String,
    pub port: u16,
    pub security: Security,
    pub username: Option<String>,
    pub has_password: bool,
    pub from_address: String,
    pub from_name: Option<String>,
    pub updated_at: OffsetDateTime,
}

/// Saisie des réglages. `password` absent garde celui en place ; vide le retire.
#[derive(Debug, Clone)]
pub struct MailInput {
    pub host: String,
    pub port: u16,
    pub security: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub from_address: String,
    pub from_name: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum MailError {
    #[error("invalid {}: {}", .0.field, .0.code.as_str())]
    Invalid(FieldError),
    #[error("mail not configured")]
    NotConfigured,
    /// Le serveur SMTP a refusé ou n'a pas répondu : cause courte, sans secret.
    #[error("mail send failed: {0}")]
    Send(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<sqlx::Error> for MailError {
    fn from(error: sqlx::Error) -> Self {
        Self::Internal(error.to_string())
    }
}

impl From<secret_box::SecretBoxError> for MailError {
    fn from(error: secret_box::SecretBoxError) -> Self {
        Self::Internal(error.to_string())
    }
}

fn invalid(field: &'static str, code: ErrorCode) -> MailError {
    MailError::Invalid(FieldError::new(field, code))
}

struct Row {
    host: String,
    port: i32,
    security: String,
    username: Option<String>,
    password: Option<Vec<u8>>,
    from_address: String,
    from_name: Option<String>,
    updated_at: OffsetDateTime,
}

async fn row(conn: &mut PgConnection) -> Result<Option<Row>, sqlx::Error> {
    sqlx::query_as!(
        Row,
        "SELECT host, port, security, username, password, from_address, from_name, updated_at
         FROM mail_settings"
    )
    .fetch_optional(conn)
    .await
}

/// Nom DNS ou adresse IP, sans espace ni caractère de contrôle.
fn checked_host(host: &str) -> Result<&str, MailError> {
    let host = host.trim();
    let valid = (1..=253).contains(&host.len())
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'));
    if !valid {
        return Err(invalid("host", ErrorCode::InvalidMailHost));
    }
    Ok(host)
}

fn checked_address(field: &'static str, address: &str) -> Result<String, MailError> {
    Email::parse(address.trim())
        .map(|email| email.as_str().to_owned())
        .map_err(|_| invalid(field, ErrorCode::InvalidMailAddress))
}

fn checked_text(
    field: &'static str,
    value: Option<&str>,
    max: usize,
) -> Result<Option<String>, MailError> {
    let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    if value.chars().count() > max || value.chars().any(char::is_control) {
        return Err(invalid(field, ErrorCode::InvalidMailField));
    }
    Ok(Some(value.to_owned()))
}

impl MailState {
    pub async fn get(&self) -> Result<Option<MailSettingsView>, MailError> {
        let mut conn = self.pool.acquire().await?;
        Ok(row(&mut conn).await?.map(|row| MailSettingsView {
            host: row.host,
            port: u16::try_from(row.port).unwrap_or(587),
            security: Security::parse(&row.security).unwrap_or(Security::StartTls),
            username: row.username,
            has_password: row.password.is_some(),
            from_address: row.from_address,
            from_name: row.from_name,
            updated_at: row.updated_at,
        }))
    }

    /// Un serveur SMTP est réglé : l'e-mail est proposable (canal, invitation).
    pub async fn configured(&self) -> Result<bool, MailError> {
        let mut conn = self.pool.acquire().await?;
        Ok(
            sqlx::query_scalar!(r#"SELECT EXISTS (SELECT 1 FROM mail_settings) AS "exists!""#)
                .fetch_one(&mut *conn)
                .await?,
        )
    }

    /// Enregistre ou remplace les réglages. Le mot de passe est chiffré ; absent, celui en place
    /// est gardé.
    pub async fn set(
        &self,
        context: &AuditContext,
        input: MailInput,
    ) -> Result<MailSettingsView, MailError> {
        let host = checked_host(&input.host)?.to_owned();
        if input.port == 0 {
            return Err(invalid("port", ErrorCode::InvalidMailField));
        }
        let security = Security::parse(input.security.trim())
            .filter(|security| *security != Security::None || self.allow_insecure)
            .ok_or_else(|| invalid("security", ErrorCode::InvalidMailField))?;
        let username = checked_text("username", input.username.as_deref(), 255)?;
        let from_address = checked_address("from_address", &input.from_address)?;
        let from_name = checked_text("from_name", input.from_name.as_deref(), 80)?;
        if let Some(password) = input.password.as_deref()
            && (password.chars().count() > 1024 || password.chars().any(char::is_control))
        {
            return Err(invalid("password", ErrorCode::InvalidMailField));
        }

        let mut tx = AuditedTx::begin(&self.pool, context).await?;
        let current = row(&mut tx).await?;
        let password = match input.password.as_deref() {
            Some("") => None,
            Some(password) => Some(secret_box::seal(&self.key, password.as_bytes())?),
            None => current.as_ref().and_then(|row| row.password.clone()),
        };
        sqlx::query!(
            "INSERT INTO mail_settings (host, port, security, username, password, from_address, from_name)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (singleton) DO UPDATE SET
                host = EXCLUDED.host, port = EXCLUDED.port, security = EXCLUDED.security,
                username = EXCLUDED.username, password = EXCLUDED.password,
                from_address = EXCLUDED.from_address, from_name = EXCLUDED.from_name,
                updated_at = now()",
            host,
            i32::from(input.port),
            security.as_str(),
            username.as_deref(),
            password.as_deref(),
            from_address,
            from_name.as_deref(),
        )
        .execute(&mut *tx)
        .await?;
        tx.commit(
            AuditEvent::success(AuditAction::MailUpdated).details(json!({
                "host": host, "port": input.port, "security": security.as_str(),
                "username": username, "from_address": from_address,
                "has_password": password.is_some(),
            })),
        )
        .await?;
        self.get()
            .await?
            .ok_or_else(|| MailError::Internal("mail settings vanished".to_owned()))
    }

    pub async fn remove(&self, context: &AuditContext) -> Result<(), MailError> {
        let mut tx = AuditedTx::begin(&self.pool, context).await?;
        let removed = sqlx::query_scalar!("DELETE FROM mail_settings RETURNING host")
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(MailError::NotConfigured)?;
        tx.commit(
            AuditEvent::success(AuditAction::MailRemoved).details(json!({ "host": removed })),
        )
        .await?;
        Ok(())
    }

    /// Nom du projet, pour les sujets et l'expéditeur.
    pub async fn brand(&self) -> Result<String, MailError> {
        let mut conn = self.pool.acquire().await?;
        Ok(SettingsRepository::get(&mut conn)
            .await?
            .project_name
            .unwrap_or_else(|| DEFAULT_BRAND.to_owned()))
    }

    /// Serveur déchiffré, le temps d'un envoi.
    async fn transport(&self) -> Result<SmtpTransport, MailError> {
        let mut conn = self.pool.acquire().await?;
        let row = row(&mut conn).await?.ok_or(MailError::NotConfigured)?;
        let from_name = match row.from_name {
            Some(name) => name,
            None => SettingsRepository::get(&mut conn)
                .await?
                .project_name
                .unwrap_or_else(|| DEFAULT_BRAND.to_owned()),
        };
        let password = match row.password {
            Some(sealed) => Some(
                String::from_utf8(secret_box::open(&self.key, &sealed)?)
                    .map_err(|error| MailError::Internal(error.to_string()))?,
            ),
            None => None,
        };
        Ok(SmtpTransport {
            host: row.host,
            port: u16::try_from(row.port).unwrap_or(587),
            security: Security::parse(&row.security).unwrap_or(Security::StartTls),
            username: row.username,
            password,
            from_address: row.from_address,
            from_name,
        })
    }

    /// Envoie un message avec les réglages du projet.
    pub async fn send(&self, message: Message) -> Result<(), MailError> {
        let transport = self.transport().await?;
        self.mailer
            .send(transport, message)
            .await
            .map_err(|error| MailError::Send(crate::notifications::domain::clean(&error, 200)))
    }

    /// E-mail de test à l'admin qui le demande ; l'issue est auditée, sans secret.
    pub async fn test(&self, context: &AuditContext, to: &str) -> Result<(), MailError> {
        let brand = self.brand().await?;
        let email = super::template::Email {
            brand: brand.clone(),
            title: "Test".to_owned(),
            paragraphs: vec![
                format!("Cet e-mail confirme que {brand} peut envoyer des messages."),
                format!("This e-mail confirms that {brand} can send messages."),
            ],
            action: self
                .panel_url
                .clone()
                .map(|url| ("Ouvrir le panel · Open the panel".to_owned(), url)),
            note: None,
        };
        let result = self
            .send(Message::from_template(vec![to.to_owned()], &email))
            .await;
        let tx = AuditedTx::begin(&self.pool, context).await?;
        let event = match &result {
            Ok(()) => AuditEvent::success(AuditAction::MailTested),
            Err(error) => AuditEvent::failure(AuditAction::MailTested)
                .details(json!({ "error": error.to_string() })),
        };
        tx.commit(event).await?;
        result
    }
}
