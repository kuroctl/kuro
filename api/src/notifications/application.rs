//! Cas d'usage des notifications (ADR-0044) : canaux et règles (audités), émission d'un
//! événement (une ligne d'envoi par règle qui correspond et par canal), worker d'envoi avec
//! reprise, limitation par canal et bail multi-instance, purge du journal.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use sqlx::PgPool;
use time::OffsetDateTime;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::domain::{
    ChannelConfig, ChannelKind, Event, EventKind, MAX_ATTEMPTS, RATE_LIMIT_PER_MINUTE, Scope,
    backoff, clean, url_hint, valid_webhook_url,
};
use super::infra::{
    ChannelFields, ChannelRecord, DeliveryRecord, NotificationRepository, RuleFields, RuleRecord,
};
use super::payload;
use super::sender::WebhookSender;
use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::auth::secret_box;
use crate::error_code::{ErrorCode, FieldError};

/// Passe du worker : toutes les 5 s ; envois réservés par lot de 50.
const DELIVER_EVERY: Duration = Duration::from_secs(5);
const BATCH: i64 = 50;
/// Conservation du journal des envois.
const KEEP_DAYS: i32 = 30;
const PURGE_EVERY: Duration = Duration::from_secs(3600);
pub const LANGUAGES: [&str; 2] = ["fr", "en"];

#[derive(Debug, thiserror::Error)]
pub enum NotificationsError {
    #[error("not found")]
    NotFound(ErrorCode),
    #[error("invalid field {}", .0.field)]
    Invalid(FieldError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("{0}")]
    Internal(String),
}

impl From<secret_box::SecretBoxError> for NotificationsError {
    fn from(error: secret_box::SecretBoxError) -> Self {
        Self::Internal(error.to_string())
    }
}

fn invalid(field: &'static str, code: ErrorCode) -> NotificationsError {
    NotificationsError::Invalid(FieldError::new(field, code))
}

#[derive(Debug, Clone)]
pub struct NotificationsState {
    pub pool: PgPool,
    /// Clé de chiffrement des configurations (celle des secrets des serveurs).
    pub key: [u8; 32],
    pub sender: Arc<dyn WebhookSender>,
    /// URL du panel, pour le lien de chaque notification ; absente : pas de lien.
    pub panel_url: Option<String>,
    /// Webhooks vers des adresses privées acceptés (Slack d'entreprise derrière un VPN…).
    pub allow_private_targets: bool,
}

/// Canal tel que rendu : jamais l'URL complète ni le secret.
#[derive(Debug, Clone)]
pub struct ChannelView {
    pub record: ChannelRecord,
    pub has_secret: bool,
}

/// Saisie d'un canal. En modification, `url` et `secret` absents gardent les valeurs en place ;
/// `secret` vide retire le secret.
#[derive(Debug, Clone, Default)]
pub struct ChannelInput {
    pub name: String,
    pub kind: String,
    pub url: Option<String>,
    pub secret: Option<String>,
    pub language: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Default)]
pub struct RuleInput {
    pub name: String,
    pub scope: String,
    pub node_id: Option<Uuid>,
    pub server_id: Option<Uuid>,
    pub events: Vec<String>,
    pub channels: Vec<Uuid>,
    pub enabled: bool,
}

/// Résultat d'un envoi de test.
#[derive(Debug, Clone)]
pub struct TestOutcome {
    pub ok: bool,
    pub status: Option<u16>,
    pub error: Option<String>,
}

impl NotificationsState {
    fn open(&self, channel: &ChannelRecord) -> Result<ChannelConfig, NotificationsError> {
        let bytes = secret_box::open(&self.key, &channel.config)?;
        serde_json::from_slice(&bytes).map_err(|e| NotificationsError::Internal(e.to_string()))
    }

    fn seal(&self, config: &ChannelConfig) -> Result<Vec<u8>, NotificationsError> {
        let bytes =
            serde_json::to_vec(config).map_err(|e| NotificationsError::Internal(e.to_string()))?;
        Ok(secret_box::seal(&self.key, &bytes)?)
    }

    pub(crate) fn view(&self, record: ChannelRecord) -> ChannelView {
        let has_secret = self
            .open(&record)
            .ok()
            .and_then(|config| config.secret)
            .is_some_and(|secret| !secret.is_empty());
        ChannelView { record, has_secret }
    }
}

fn audit_channel(record: &ChannelRecord) -> Value {
    json!({
        "channel": record.id,
        "server_id": record.server_id,
        "name": record.name,
        "kind": record.kind,
        "url_hint": record.url_hint,
        "language": record.language,
        "enabled": record.enabled,
    })
}

fn audit_rule(record: &RuleRecord) -> Value {
    json!({
        "rule": record.id,
        "name": record.name,
        "scope": record.scope,
        "node_id": record.node_id,
        "server_id": record.server_id,
        "events": record.events,
        "channels": record.channels,
        "enabled": record.enabled,
    })
}

fn validate_name(name: &str) -> Result<String, NotificationsError> {
    let name = name.trim();
    if !crate::validation::is_display_text(name, 1, 80) {
        return Err(invalid("name", ErrorCode::InvalidNotificationName));
    }
    Ok(name.to_owned())
}

fn validate_language(language: &str) -> Result<String, NotificationsError> {
    LANGUAGES
        .iter()
        .find(|l| **l == language)
        .map(|l| (*l).to_owned())
        .ok_or_else(|| invalid("language", ErrorCode::InvalidNotificationLanguage))
}

pub async fn channels(state: &NotificationsState) -> Result<Vec<ChannelView>, NotificationsError> {
    let mut conn = state.pool.acquire().await?;
    let records = NotificationRepository::channels(&mut conn).await?;
    Ok(records
        .into_iter()
        .map(|record| state.view(record))
        .collect())
}

pub async fn create_channel(
    state: &NotificationsState,
    context: &AuditContext,
    input: &ChannelInput,
) -> Result<ChannelView, NotificationsError> {
    create_channel_for(state, context, None, input).await
}

/// Crée un canal, partagé (`server_id` absent) ou propre à un serveur (abonnement).
pub async fn create_channel_for(
    state: &NotificationsState,
    context: &AuditContext,
    server_id: Option<Uuid>,
    input: &ChannelInput,
) -> Result<ChannelView, NotificationsError> {
    let name = validate_name(&input.name)?;
    let kind = ChannelKind::parse(&input.kind)
        .ok_or_else(|| invalid("kind", ErrorCode::InvalidNotificationChannelKind))?;
    let language = validate_language(&input.language)?;
    let url = input
        .url
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .ok_or_else(|| invalid("url", ErrorCode::InvalidWebhookUrl))?;
    if !valid_webhook_url(url) {
        return Err(invalid("url", ErrorCode::InvalidWebhookUrl));
    }
    let secret = input
        .secret
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| clean(s, 256));
    let config = ChannelConfig {
        url: url.to_owned(),
        secret,
    };
    let sealed = state.seal(&config)?;
    let id = Uuid::now_v7();
    let hint = url_hint(url);
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    NotificationRepository::insert_channel(
        &mut tx,
        id,
        context.actor.id,
        &ChannelFields {
            name: &name,
            kind: kind.as_str(),
            config: &sealed,
            url_hint: &hint,
            language: &language,
            enabled: input.enabled,
            server_id,
        },
    )
    .await?;
    let record = NotificationRepository::channel(&mut tx, id)
        .await?
        .ok_or(NotificationsError::Internal("channel vanished".to_owned()))?;
    tx.commit(
        AuditEvent::success(AuditAction::NotificationChannelCreated)
            .target("notification_channel", id)
            .details(audit_channel(&record)),
    )
    .await?;
    Ok(state.view(record))
}

pub async fn update_channel(
    state: &NotificationsState,
    context: &AuditContext,
    id: Uuid,
    input: &ChannelInput,
) -> Result<ChannelView, NotificationsError> {
    let name = validate_name(&input.name)?;
    let kind = ChannelKind::parse(&input.kind)
        .ok_or_else(|| invalid("kind", ErrorCode::InvalidNotificationChannelKind))?;
    let language = validate_language(&input.language)?;
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let current =
        NotificationRepository::channel(&mut tx, id)
            .await?
            .ok_or(NotificationsError::NotFound(
                ErrorCode::NotificationChannelNotFound,
            ))?;
    let mut config = state.open(&current)?;
    if let Some(url) = input
        .url
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
    {
        if !valid_webhook_url(url) {
            return Err(invalid("url", ErrorCode::InvalidWebhookUrl));
        }
        config.url = url.to_owned();
    }
    if let Some(secret) = input.secret.as_deref().map(str::trim) {
        config.secret = (!secret.is_empty()).then(|| clean(secret, 256));
    }
    let sealed = state.seal(&config)?;
    let hint = url_hint(&config.url);
    NotificationRepository::update_channel(
        &mut tx,
        id,
        &ChannelFields {
            name: &name,
            kind: kind.as_str(),
            config: &sealed,
            url_hint: &hint,
            language: &language,
            enabled: input.enabled,
            server_id: current.server_id,
        },
    )
    .await?;
    let record = NotificationRepository::channel(&mut tx, id)
        .await?
        .ok_or(NotificationsError::Internal("channel vanished".to_owned()))?;
    tx.commit(
        AuditEvent::success(AuditAction::NotificationChannelUpdated)
            .target("notification_channel", id)
            .details(audit_channel(&record)),
    )
    .await?;
    Ok(state.view(record))
}

pub async fn delete_channel(
    state: &NotificationsState,
    context: &AuditContext,
    id: Uuid,
) -> Result<(), NotificationsError> {
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let record =
        NotificationRepository::channel(&mut tx, id)
            .await?
            .ok_or(NotificationsError::NotFound(
                ErrorCode::NotificationChannelNotFound,
            ))?;
    NotificationRepository::delete_channel(&mut tx, id).await?;
    tx.commit(
        AuditEvent::success(AuditAction::NotificationChannelDeleted)
            .target("notification_channel", id)
            .details(audit_channel(&record)),
    )
    .await?;
    Ok(())
}

/// Envoie un événement `test` sur le canal, tout de suite, et dit ce que le destinataire a
/// répondu. Journalisé, sans passer par la file des envois.
pub async fn test_channel(
    state: &NotificationsState,
    context: &AuditContext,
    id: Uuid,
) -> Result<TestOutcome, NotificationsError> {
    let record = {
        let mut conn = state.pool.acquire().await?;
        NotificationRepository::channel(&mut conn, id)
            .await?
            .ok_or(NotificationsError::NotFound(
                ErrorCode::NotificationChannelNotFound,
            ))?
    };
    let config = state.open(&record)?;
    let kind = ChannelKind::parse(&record.kind)
        .ok_or_else(|| NotificationsError::Internal("unknown channel kind".to_owned()))?;
    let event = Event::now(EventKind::Test);
    let outgoing = payload::build(
        kind,
        &config,
        &event,
        &record.language,
        Uuid::now_v7(),
        state.panel_url.as_deref(),
    );
    let result = state
        .sender
        .send(outgoing, state.allow_private_targets)
        .await;
    let outcome = match result {
        Ok(status) => TestOutcome {
            ok: true,
            status: Some(status),
            error: None,
        },
        Err(error) => TestOutcome {
            ok: false,
            status: None,
            error: Some(clean(&error, 200)),
        },
    };
    let event = if outcome.ok {
        AuditEvent::success(AuditAction::NotificationChannelTested)
    } else {
        AuditEvent::failure(AuditAction::NotificationChannelTested)
    };
    crate::audit::application::record(
        &state.pool,
        context,
        event
            .target("notification_channel", id)
            .details(json!({ "channel": id, "ok": outcome.ok, "error": outcome.error })),
    )
    .await?;
    Ok(outcome)
}

pub async fn rules(state: &NotificationsState) -> Result<Vec<RuleRecord>, NotificationsError> {
    let mut conn = state.pool.acquire().await?;
    Ok(NotificationRepository::rules(&mut conn).await?)
}

/// Règle validée : portée cohérente et existante, événements connus, canaux existants.
struct ValidatedRule {
    name: String,
    scope: Scope,
    events: Vec<String>,
    channels: Vec<Uuid>,
}

async fn validate_rule(
    conn: &mut sqlx::PgConnection,
    input: &RuleInput,
) -> Result<ValidatedRule, NotificationsError> {
    let name = validate_name(&input.name)?;
    let scope = match (input.scope.as_str(), input.node_id, input.server_id) {
        ("all", None, None) => Scope::All,
        ("node", Some(node), None) => {
            crate::nodes::infra::NodeRepository::get(conn, node)
                .await?
                .ok_or_else(|| invalid("node_id", ErrorCode::InvalidNotificationScope))?;
            Scope::Node(node)
        }
        ("server", None, Some(server)) => {
            crate::servers::infra::ServerRepository::get(conn, server)
                .await?
                .ok_or_else(|| invalid("server_id", ErrorCode::InvalidNotificationScope))?;
            Scope::Server(server)
        }
        _ => return Err(invalid("scope", ErrorCode::InvalidNotificationScope)),
    };
    let events: BTreeSet<String> = input
        .events
        .iter()
        .map(|event| {
            EventKind::parse(event)
                .filter(|kind| *kind != EventKind::Test)
                .map(|kind| kind.as_str().to_owned())
                .ok_or_else(|| invalid("events", ErrorCode::InvalidNotificationEvents))
        })
        .collect::<Result<_, _>>()?;
    if events.is_empty() {
        return Err(invalid("events", ErrorCode::InvalidNotificationEvents));
    }
    let channels: BTreeSet<Uuid> = input.channels.iter().copied().collect();
    if channels.is_empty() {
        return Err(invalid("channels", ErrorCode::InvalidNotificationChannels));
    }
    let wanted: Vec<Uuid> = channels.iter().copied().collect();
    let existing = NotificationRepository::existing_channels(conn, &wanted).await?;
    if existing.len() != wanted.len() {
        return Err(invalid("channels", ErrorCode::InvalidNotificationChannels));
    }
    Ok(ValidatedRule {
        name,
        scope,
        events: events.into_iter().collect(),
        channels: wanted,
    })
}

fn rule_fields(validated: &ValidatedRule, enabled: bool) -> RuleFields<'_> {
    RuleFields {
        name: &validated.name,
        scope: validated.scope.kind(),
        node_id: match validated.scope {
            Scope::Node(id) => Some(id),
            _ => None,
        },
        server_id: match validated.scope {
            Scope::Server(id) => Some(id),
            _ => None,
        },
        events: &validated.events,
        channels: &validated.channels,
        enabled,
    }
}

pub async fn create_rule(
    state: &NotificationsState,
    context: &AuditContext,
    input: &RuleInput,
) -> Result<RuleRecord, NotificationsError> {
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let validated = validate_rule(&mut tx, input).await?;
    let id = Uuid::now_v7();
    NotificationRepository::insert_rule(
        &mut tx,
        id,
        context.actor.id,
        &rule_fields(&validated, input.enabled),
    )
    .await?;
    let record = NotificationRepository::rule(&mut tx, id)
        .await?
        .ok_or(NotificationsError::Internal("rule vanished".to_owned()))?;
    tx.commit(
        AuditEvent::success(AuditAction::NotificationRuleCreated)
            .target("notification_rule", id)
            .details(audit_rule(&record)),
    )
    .await?;
    Ok(record)
}

pub async fn update_rule(
    state: &NotificationsState,
    context: &AuditContext,
    id: Uuid,
    input: &RuleInput,
) -> Result<RuleRecord, NotificationsError> {
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    NotificationRepository::rule(&mut tx, id)
        .await?
        .ok_or(NotificationsError::NotFound(
            ErrorCode::NotificationRuleNotFound,
        ))?;
    let validated = validate_rule(&mut tx, input).await?;
    NotificationRepository::update_rule(&mut tx, id, &rule_fields(&validated, input.enabled))
        .await?;
    let record = NotificationRepository::rule(&mut tx, id)
        .await?
        .ok_or(NotificationsError::Internal("rule vanished".to_owned()))?;
    tx.commit(
        AuditEvent::success(AuditAction::NotificationRuleUpdated)
            .target("notification_rule", id)
            .details(audit_rule(&record)),
    )
    .await?;
    Ok(record)
}

pub async fn delete_rule(
    state: &NotificationsState,
    context: &AuditContext,
    id: Uuid,
) -> Result<(), NotificationsError> {
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let record =
        NotificationRepository::rule(&mut tx, id)
            .await?
            .ok_or(NotificationsError::NotFound(
                ErrorCode::NotificationRuleNotFound,
            ))?;
    NotificationRepository::delete_rule(&mut tx, id).await?;
    tx.commit(
        AuditEvent::success(AuditAction::NotificationRuleDeleted)
            .target("notification_rule", id)
            .details(audit_rule(&record)),
    )
    .await?;
    Ok(())
}

pub async fn deliveries(
    state: &NotificationsState,
    limit: i64,
) -> Result<Vec<DeliveryRecord>, NotificationsError> {
    let mut conn = state.pool.acquire().await?;
    Ok(NotificationRepository::deliveries(&mut conn, limit.clamp(1, 500)).await?)
}

/// Émet un événement : une ligne d'envoi `pending` par canal activé d'une règle activée qui le
/// couvre (un même canal visé par plusieurs règles ne reçoit qu'un message). Rend le nombre
/// d'envois créés. N'échoue jamais le cas d'usage appelant : l'erreur est rendue, à journaliser.
pub async fn emit(pool: &PgPool, event: Event) -> Result<usize, sqlx::Error> {
    let mut conn = pool.acquire().await?;
    let rules = NotificationRepository::enabled_rules(&mut conn).await?;
    let kind = event.kind.as_str();
    let mut targets: Vec<(Uuid, Uuid)> = Vec::new();
    let mut seen: BTreeSet<Uuid> = BTreeSet::new();
    for rule in rules {
        let scope = match (rule.scope.as_str(), rule.node_id, rule.server_id) {
            ("node", Some(node), _) => Scope::Node(node),
            ("server", _, Some(server)) => Scope::Server(server),
            _ => Scope::All,
        };
        if !rule.events.iter().any(|e| e == kind) || !scope.covers(&event) {
            continue;
        }
        for channel in NotificationRepository::enabled_channels(&mut conn, &rule.channels).await? {
            if seen.insert(channel) {
                targets.push((rule.id, channel));
            }
        }
    }
    if targets.is_empty() {
        return Ok(0);
    }
    let value = serde_json::to_value(&event).unwrap_or(Value::Null);
    for (rule, channel) in &targets {
        NotificationRepository::insert_delivery(&mut conn, Uuid::now_v7(), *rule, *channel, &value)
            .await?;
    }
    Ok(targets.len())
}

/// Émission depuis un cas d'usage : l'échec est journalisé, jamais remonté (une notification
/// ne doit pas faire échouer ce qu'elle annonce).
pub fn emit_detached(pool: &PgPool, event: Event) {
    let pool = pool.clone();
    tokio::spawn(async move {
        if let Err(error) = emit(&pool, event).await {
            tracing::error!(%error, "failed to queue a notification");
        }
    });
}

/// Une passe du worker : envois dus réservés, envoyés, puis marqués. Rend le nombre d'envois
/// tentés.
pub async fn deliver_due(state: &NotificationsState) -> Result<usize, NotificationsError> {
    let claimed = {
        let mut conn = state.pool.acquire().await?;
        NotificationRepository::claim_due(&mut conn, BATCH).await?
    };
    let mut attempted = 0;
    for delivery in claimed {
        deliver_one(state, &delivery).await?;
        attempted += 1;
    }
    Ok(attempted)
}

async fn deliver_one(
    state: &NotificationsState,
    delivery: &DeliveryRecord,
) -> Result<(), NotificationsError> {
    let mut conn = state.pool.acquire().await?;
    let Some(channel) = NotificationRepository::channel(&mut conn, delivery.channel_id).await?
    else {
        return Ok(()); // canal supprimé : l'envoi est parti avec lui (cascade)
    };
    if !channel.enabled {
        NotificationRepository::mark_ended(
            &mut conn,
            delivery.id,
            "dropped",
            delivery.attempts,
            "channel_disabled",
        )
        .await?;
        return Ok(());
    }
    // Limitation par canal : au-delà du débit, l'envoi attend la minute suivante sans compter
    // de tentative.
    if NotificationRepository::sent_last_minute(&mut conn, channel.id).await?
        >= RATE_LIMIT_PER_MINUTE
    {
        NotificationRepository::postpone(
            &mut conn,
            delivery.id,
            OffsetDateTime::now_utc() + time::Duration::seconds(60),
        )
        .await?;
        return Ok(());
    }
    let Some(event) = payload::event_from(&delivery.event) else {
        NotificationRepository::mark_ended(
            &mut conn,
            delivery.id,
            "dropped",
            delivery.attempts,
            "unreadable_event",
        )
        .await?;
        return Ok(());
    };
    let kind = ChannelKind::parse(&channel.kind).unwrap_or(ChannelKind::Webhook);
    let config = match state.open(&channel) {
        Ok(config) => config,
        Err(error) => {
            NotificationRepository::mark_ended(
                &mut conn,
                delivery.id,
                "failed",
                delivery.attempts,
                &clean(&error.to_string(), 200),
            )
            .await?;
            return Ok(());
        }
    };
    let outgoing = payload::build(
        kind,
        &config,
        &event,
        &channel.language,
        delivery.id,
        state.panel_url.as_deref(),
    );
    drop(conn);
    let result = state
        .sender
        .send(outgoing, state.allow_private_targets)
        .await;
    let attempts = delivery.attempts + 1;
    let mut conn = state.pool.acquire().await?;
    match result {
        Ok(_) => NotificationRepository::mark_sent(&mut conn, delivery.id, attempts).await?,
        Err(error) => {
            let error = clean(&error, 200);
            match backoff(attempts) {
                Some(wait) if attempts < MAX_ATTEMPTS => {
                    NotificationRepository::mark_retry(
                        &mut conn,
                        delivery.id,
                        attempts,
                        OffsetDateTime::now_utc() + wait,
                        &error,
                    )
                    .await?;
                }
                _ => {
                    NotificationRepository::mark_ended(
                        &mut conn,
                        delivery.id,
                        "failed",
                        attempts,
                        &error,
                    )
                    .await?;
                }
            }
        }
    }
    Ok(())
}

/// Boucle de l'API : envois toutes les 5 s, purge du journal toutes les heures.
pub async fn run_forever(state: NotificationsState, shutdown: CancellationToken) {
    let mut deliver = tokio::time::interval(DELIVER_EVERY);
    deliver.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut purge = tokio::time::interval(PURGE_EVERY);
    purge.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = shutdown.cancelled() => break,
            _ = deliver.tick() => {
                if let Err(error) = deliver_due(&state).await {
                    tracing::error!(%error, "notification delivery pass failed");
                }
            }
            _ = purge.tick() => {
                let purged = async {
                    let mut conn = state.pool.acquire().await?;
                    NotificationRepository::purge(&mut conn, KEEP_DAYS).await
                }
                .await;
                match purged {
                    Ok(0) | Err(_) if false => {}
                    Ok(count) if count > 0 => tracing::info!(count, "old notification deliveries purged"),
                    Ok(_) => {}
                    Err(error) => tracing::error!(%error, "failed to purge notification deliveries"),
                }
            }
        }
    }
}
