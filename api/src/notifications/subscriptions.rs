//! Abonnements d'un serveur (ADR-0044, amendement) : vus depuis la page du serveur, une
//! destination (canal possédé par le serveur, ou canal partagé des paramètres) et les
//! événements de ce serveur. En interne : une règle de portée serveur et son unique canal.

use serde_json::json;
use uuid::Uuid;

use super::application::{
    ChannelInput, ChannelView, NotificationsError, NotificationsState, TestOutcome,
    create_channel_for, delete_channel, test_channel, update_channel,
};
use super::domain::{ChannelKind, EventKind};
use super::infra::{DeliveryRecord, NotificationRepository, RuleFields, RuleRecord};
use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::error_code::{ErrorCode, FieldError};
use crate::servers::infra::ServerRepository;

/// Événements qu'un abonnement de serveur peut viser : ceux du serveur, jamais ceux du nœud.
pub const SERVER_EVENTS: [EventKind; 5] = [
    EventKind::ServerCrashed,
    EventKind::ServerScheduleFailed,
    EventKind::ServerBackupFailed,
    EventKind::ServerUpgradeFailed,
    EventKind::ServerDiskHigh,
];

/// Saisie d'un abonnement : `channel_id` pour un canal partagé, sinon une destination propre
/// (`kind`, `url`, `secret`, `language`). En modification, `url` et `secret` absents gardent
/// les valeurs en place.
#[derive(Debug, Clone, Default)]
pub struct SubscriptionInput {
    pub name: String,
    pub events: Vec<String>,
    pub enabled: bool,
    pub channel_id: Option<Uuid>,
    pub kind: Option<String>,
    pub url: Option<String>,
    pub secret: Option<String>,
    pub language: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SubscriptionView {
    pub rule: RuleRecord,
    pub channel: ChannelView,
    /// Canal possédé par le serveur (sinon : canal partagé des paramètres).
    pub owned: bool,
}

fn invalid(field: &'static str, code: ErrorCode) -> NotificationsError {
    NotificationsError::Invalid(FieldError::new(field, code))
}

async fn ensure_server(
    conn: &mut sqlx::PgConnection,
    server_id: Uuid,
) -> Result<(), NotificationsError> {
    ServerRepository::get(conn, server_id)
        .await?
        .map(|_| ())
        .ok_or(NotificationsError::NotFound(ErrorCode::NotFound))
}

fn validate_events(events: &[String]) -> Result<Vec<String>, NotificationsError> {
    let mut out: Vec<String> = Vec::new();
    for event in events {
        let kind = EventKind::parse(event)
            .filter(|kind| SERVER_EVENTS.contains(kind))
            .ok_or_else(|| invalid("events", ErrorCode::InvalidNotificationEvents))?;
        if !out.iter().any(|e| e == kind.as_str()) {
            out.push(kind.as_str().to_owned());
        }
    }
    if out.is_empty() {
        return Err(invalid("events", ErrorCode::InvalidNotificationEvents));
    }
    Ok(out)
}

async fn view(
    state: &NotificationsState,
    conn: &mut sqlx::PgConnection,
    rule: RuleRecord,
) -> Result<Option<SubscriptionView>, NotificationsError> {
    let Some(channel_id) = rule.channels.first().copied() else {
        return Ok(None);
    };
    let Some(channel) = NotificationRepository::channel(conn, channel_id).await? else {
        return Ok(None);
    };
    let owned = channel.server_id.is_some();
    Ok(Some(SubscriptionView {
        rule,
        channel: state.view(channel),
        owned,
    }))
}

pub async fn list(
    state: &NotificationsState,
    server_id: Uuid,
) -> Result<Vec<SubscriptionView>, NotificationsError> {
    let mut conn = state.pool.acquire().await?;
    ensure_server(&mut conn, server_id).await?;
    let rules = NotificationRepository::rules_of_server(&mut conn, server_id).await?;
    let mut out = Vec::with_capacity(rules.len());
    for rule in rules {
        if let Some(subscription) = view(state, &mut conn, rule).await? {
            out.push(subscription);
        }
    }
    Ok(out)
}

pub async fn get(
    state: &NotificationsState,
    server_id: Uuid,
    id: Uuid,
) -> Result<SubscriptionView, NotificationsError> {
    let mut conn = state.pool.acquire().await?;
    let rule = NotificationRepository::rule(&mut conn, id)
        .await?
        .filter(|rule| rule.scope == "server" && rule.server_id == Some(server_id))
        .ok_or(NotificationsError::NotFound(
            ErrorCode::NotificationRuleNotFound,
        ))?;
    view(state, &mut conn, rule)
        .await?
        .ok_or(NotificationsError::NotFound(
            ErrorCode::NotificationRuleNotFound,
        ))
}

/// Canal partagé demandé : doit exister, être partagé, et l'appelant doit y avoir accès.
async fn shared_channel(
    conn: &mut sqlx::PgConnection,
    id: Uuid,
    may_use_shared: bool,
) -> Result<Uuid, NotificationsError> {
    let channel = NotificationRepository::channel(conn, id)
        .await?
        .filter(|channel| channel.server_id.is_none())
        .ok_or_else(|| invalid("channel_id", ErrorCode::InvalidNotificationChannels))?;
    if !may_use_shared {
        return Err(invalid(
            "channel_id",
            ErrorCode::InvalidNotificationChannels,
        ));
    }
    Ok(channel.id)
}

fn channel_input(input: &SubscriptionInput, fallback_language: &str) -> ChannelInput {
    ChannelInput {
        name: input.name.clone(),
        kind: input
            .kind
            .clone()
            .unwrap_or_else(|| ChannelKind::Webhook.as_str().to_owned()),
        url: input.url.clone(),
        secret: input.secret.clone(),
        language: input
            .language
            .clone()
            .unwrap_or_else(|| fallback_language.to_owned()),
        enabled: true,
    }
}

/// Crée l'abonnement : la destination (canal propre créé, ou canal partagé), puis la règle.
pub async fn create(
    state: &NotificationsState,
    context: &AuditContext,
    server_id: Uuid,
    input: &SubscriptionInput,
    may_use_shared: bool,
) -> Result<SubscriptionView, NotificationsError> {
    let events = validate_events(&input.events)?;
    let name = input.name.trim();
    if !crate::validation::is_display_text(name, 1, 80) {
        return Err(invalid("name", ErrorCode::InvalidNotificationName));
    }
    {
        let mut conn = state.pool.acquire().await?;
        ensure_server(&mut conn, server_id).await?;
    }
    let channel_id = match input.channel_id {
        Some(id) => {
            let mut conn = state.pool.acquire().await?;
            shared_channel(&mut conn, id, may_use_shared).await?
        }
        None => {
            create_channel_for(state, context, Some(server_id), &channel_input(input, "fr"))
                .await?
                .record
                .id
        }
    };
    let id = Uuid::now_v7();
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    NotificationRepository::insert_rule(
        &mut tx,
        id,
        context.actor.id,
        &RuleFields {
            name,
            scope: "server",
            node_id: None,
            server_id: Some(server_id),
            events: &events,
            channels: &[channel_id],
            enabled: input.enabled,
        },
    )
    .await?;
    tx.commit(
        AuditEvent::success(AuditAction::NotificationRuleCreated)
            .target("server", server_id)
            .details(json!({ "rule": id, "name": name, "events": events, "channel": channel_id })),
    )
    .await?;
    get(state, server_id, id).await
}

/// Remplace l'abonnement. Passer d'une destination propre à un canal partagé supprime le canal
/// propre ; l'inverse en crée un.
pub async fn update(
    state: &NotificationsState,
    context: &AuditContext,
    server_id: Uuid,
    id: Uuid,
    input: &SubscriptionInput,
    may_use_shared: bool,
) -> Result<SubscriptionView, NotificationsError> {
    let current = get(state, server_id, id).await?;
    let events = validate_events(&input.events)?;
    let name = input.name.trim();
    if !crate::validation::is_display_text(name, 1, 80) {
        return Err(invalid("name", ErrorCode::InvalidNotificationName));
    }
    let channel_id = match (input.channel_id, current.owned) {
        // Canal partagé demandé : vérifié ; le canal propre éventuel n'a plus de raison d'être.
        (Some(shared), owned) => {
            let mut conn = state.pool.acquire().await?;
            let shared = shared_channel(&mut conn, shared, may_use_shared).await?;
            drop(conn);
            if owned {
                delete_channel(state, context, current.channel.record.id).await?;
            }
            shared
        }
        // Destination propre mise à jour (URL et secret absents : gardés).
        (None, true) => {
            update_channel(
                state,
                context,
                current.channel.record.id,
                &channel_input(input, &current.channel.record.language),
            )
            .await?;
            current.channel.record.id
        }
        // D'un canal partagé vers une destination propre.
        (None, false) => {
            create_channel_for(state, context, Some(server_id), &channel_input(input, "fr"))
                .await?
                .record
                .id
        }
    };
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    NotificationRepository::update_rule(
        &mut tx,
        id,
        &RuleFields {
            name,
            scope: "server",
            node_id: None,
            server_id: Some(server_id),
            events: &events,
            channels: &[channel_id],
            enabled: input.enabled,
        },
    )
    .await?;
    tx.commit(
        AuditEvent::success(AuditAction::NotificationRuleUpdated)
            .target("server", server_id)
            .details(json!({ "rule": id, "name": name, "events": events, "channel": channel_id })),
    )
    .await?;
    get(state, server_id, id).await
}

/// Supprime l'abonnement et, s'il était propre au serveur, son canal.
pub async fn delete(
    state: &NotificationsState,
    context: &AuditContext,
    server_id: Uuid,
    id: Uuid,
) -> Result<(), NotificationsError> {
    let current = get(state, server_id, id).await?;
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    NotificationRepository::delete_rule(&mut tx, id).await?;
    tx.commit(
        AuditEvent::success(AuditAction::NotificationRuleDeleted)
            .target("server", server_id)
            .details(json!({ "rule": id, "name": current.rule.name })),
    )
    .await?;
    if current.owned {
        delete_channel(state, context, current.channel.record.id).await?;
    }
    Ok(())
}

pub async fn test(
    state: &NotificationsState,
    context: &AuditContext,
    server_id: Uuid,
    id: Uuid,
) -> Result<TestOutcome, NotificationsError> {
    let current = get(state, server_id, id).await?;
    test_channel(state, context, current.channel.record.id).await
}

/// Journal des envois de ce serveur.
pub async fn deliveries(
    state: &NotificationsState,
    server_id: Uuid,
    limit: i64,
) -> Result<Vec<DeliveryRecord>, NotificationsError> {
    let mut conn = state.pool.acquire().await?;
    ensure_server(&mut conn, server_id).await?;
    Ok(
        NotificationRepository::deliveries_of_server(&mut conn, server_id, limit.clamp(1, 500))
            .await?,
    )
}
