//! `/settings/notifications` : canaux, règles et journal des envois (ADR-0044). Lecture :
//! `system.notifications.read` ; écriture et test : `system.notifications.manage`.

use axum::Router;
use axum::extract::{FromRef, State};
use axum::http::StatusCode;
use axum::routing::{get as route_get, post as route_post};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use utoipa::{IntoParams, OpenApi, ToSchema};
use uuid::Uuid;

use super::application::{
    self as notifications, ChannelInput, ChannelView, NotificationsError, NotificationsState,
    RuleInput,
};
use super::domain::EventKind;
use super::infra::{DeliveryRecord, RuleRecord};
use super::subscriptions::{self, SERVER_EVENTS, SubscriptionInput, SubscriptionView};
use crate::http::error::{ApiError, Problem, error_response, internal_error};
use crate::http::extract::{Json, Path, Query};
use crate::iam::application::IamState;
use crate::iam::domain::Permission;
use crate::iam::http::AdminSession;

#[derive(OpenApi)]
#[openapi(
    paths(events, list_channels, create_channel, update_channel, delete_channel, test_channel,
          list_rules, create_rule, update_rule, delete_rule, list_deliveries,
          list_subscriptions, create_subscription, update_subscription, delete_subscription,
          test_subscription, server_deliveries),
    tags((name = "admin-notifications", description = "Notifications et webhooks (ADR-0044)"))
)]
pub struct NotificationsApi;

pub fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    IamState: FromRef<S>,
    NotificationsState: FromRef<S>,
{
    Router::new()
        .route("/settings/notifications/events", route_get(events))
        .route(
            "/settings/notifications/channels",
            route_get(list_channels).post(create_channel),
        )
        .route(
            "/settings/notifications/channels/{id}",
            axum::routing::put(update_channel).delete(delete_channel),
        )
        .route(
            "/settings/notifications/channels/{id}/test",
            route_post(test_channel),
        )
        .route(
            "/settings/notifications/rules",
            route_get(list_rules).post(create_rule),
        )
        .route(
            "/settings/notifications/rules/{id}",
            axum::routing::put(update_rule).delete(delete_rule),
        )
        .route(
            "/settings/notifications/deliveries",
            route_get(list_deliveries),
        )
        .route(
            "/servers/{id}/notifications",
            route_get(list_subscriptions).post(create_subscription),
        )
        .route(
            "/servers/{id}/notifications/deliveries",
            route_get(server_deliveries),
        )
        .route(
            "/servers/{id}/notifications/{sub}",
            axum::routing::put(update_subscription).delete(delete_subscription),
        )
        .route(
            "/servers/{id}/notifications/{sub}/test",
            route_post(test_subscription),
        )
}

impl From<NotificationsError> for ApiError {
    fn from(error: NotificationsError) -> Self {
        match error {
            NotificationsError::NotFound(code) => {
                Self::from(error_response(StatusCode::NOT_FOUND, code))
            }
            NotificationsError::Invalid(field) => Self::from(Problem::invalid(field)),
            NotificationsError::Database(_) | NotificationsError::Internal(_) => {
                Self::from(internal_error(&error, "notifications request failed"))
            }
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ChannelDto {
    pub id: Uuid,
    pub name: String,
    /// `webhook`, `discord` ou `slack`.
    pub kind: String,
    /// URL tronquée (hôte et début du chemin) : l'URL complète n'est jamais rendue.
    pub url_hint: String,
    /// Un secret HMAC est enregistré (webhook générique).
    pub has_secret: bool,
    /// `fr` ou `en` : langue des messages envoyés.
    pub language: String,
    pub enabled: bool,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: OffsetDateTime,
}

fn channel_dto(view: ChannelView) -> ChannelDto {
    ChannelDto {
        id: view.record.id,
        name: view.record.name,
        kind: view.record.kind,
        url_hint: view.record.url_hint,
        has_secret: view.has_secret,
        language: view.record.language,
        enabled: view.record.enabled,
        created_at: view.record.created_at,
        updated_at: view.record.updated_at,
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ChannelRequest {
    pub name: String,
    /// `webhook`, `discord` ou `slack`.
    pub kind: String,
    /// URL `https://` du webhook. Obligatoire à la création ; absente en modification, celle
    /// enregistrée est gardée.
    #[serde(default)]
    pub url: Option<String>,
    /// Secret HMAC (webhook générique). Absent en modification : gardé ; vide : retiré.
    #[serde(default)]
    pub secret: Option<String>,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_language() -> String {
    "fr".to_owned()
}

fn default_true() -> bool {
    true
}

impl From<ChannelRequest> for ChannelInput {
    fn from(request: ChannelRequest) -> Self {
        Self {
            name: request.name,
            kind: request.kind,
            url: request.url,
            secret: request.secret,
            language: request.language,
            enabled: request.enabled,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RuleDto {
    pub id: Uuid,
    pub name: String,
    /// `all`, `node` ou `server`.
    pub scope: String,
    pub node_id: Option<Uuid>,
    pub server_id: Option<Uuid>,
    /// Types d'événements (`server.crashed`…).
    pub events: Vec<String>,
    pub channels: Vec<Uuid>,
    pub enabled: bool,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: OffsetDateTime,
}

fn rule_dto(record: RuleRecord) -> RuleDto {
    RuleDto {
        id: record.id,
        name: record.name,
        scope: record.scope,
        node_id: record.node_id,
        server_id: record.server_id,
        events: record.events,
        channels: record.channels,
        enabled: record.enabled,
        created_at: record.created_at,
        updated_at: record.updated_at,
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RuleRequest {
    pub name: String,
    /// `all`, `node` (avec `node_id`) ou `server` (avec `server_id`).
    pub scope: String,
    #[serde(default)]
    pub node_id: Option<Uuid>,
    #[serde(default)]
    pub server_id: Option<Uuid>,
    /// Au moins un type d'événement, parmi `/settings/notifications/events`.
    pub events: Vec<String>,
    /// Au moins un canal.
    pub channels: Vec<Uuid>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl From<RuleRequest> for RuleInput {
    fn from(request: RuleRequest) -> Self {
        Self {
            name: request.name,
            scope: request.scope,
            node_id: request.node_id,
            server_id: request.server_id,
            events: request.events,
            channels: request.channels,
            enabled: request.enabled,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DeliveryDto {
    pub id: Uuid,
    pub rule_id: Option<Uuid>,
    pub channel_id: Uuid,
    /// Événement tel qu'émis (`kind`, `at`, `server`, `node`, `reason`).
    #[schema(value_type = Object)]
    pub event: Value,
    /// `pending`, `sent`, `failed` ou `dropped`.
    pub status: String,
    pub attempts: i32,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub next_attempt_at: OffsetDateTime,
    pub last_error: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub sent_at: Option<OffsetDateTime>,
}

fn delivery_dto(record: DeliveryRecord) -> DeliveryDto {
    DeliveryDto {
        id: record.id,
        rule_id: record.rule_id,
        channel_id: record.channel_id,
        event: record.event,
        status: record.status,
        attempts: record.attempts,
        next_attempt_at: record.next_attempt_at,
        last_error: record.last_error,
        created_at: record.created_at,
        sent_at: record.sent_at,
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TestDto {
    pub ok: bool,
    /// Statut HTTP renvoyé par le destinataire, en cas de succès.
    pub status: Option<u16>,
    /// Cause de l'échec (code stable ou message court).
    pub error: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct DeliveriesQuery {
    /// Nombre d'envois, les plus récents d'abord (50 par défaut, 500 au plus).
    #[serde(default)]
    pub limit: Option<i64>,
}

/// Types d'événements qu'une règle peut viser. Permission : `system.notifications.read`.
#[utoipa::path(get, path = "/api/admin/v1/settings/notifications/events", tag = "admin-notifications",
    security(("bearer" = [])), responses((status = 200, body = [String]), (status = 403, body = Problem)))]
pub async fn events(
    State(iam): State<IamState>,
    session: AdminSession,
) -> Result<Json<Vec<&'static str>>, ApiError> {
    session
        .require(&iam, Permission::SystemNotificationsRead)
        .await?;
    Ok(Json(
        EventKind::RULE_EVENTS
            .into_iter()
            .map(EventKind::as_str)
            .collect(),
    ))
}

/// Canaux configurés. Permission : `system.notifications.read`.
#[utoipa::path(get, path = "/api/admin/v1/settings/notifications/channels", tag = "admin-notifications",
    security(("bearer" = [])), responses((status = 200, body = [ChannelDto]), (status = 403, body = Problem)))]
pub async fn list_channels(
    State(iam): State<IamState>,
    State(state): State<NotificationsState>,
    session: AdminSession,
) -> Result<Json<Vec<ChannelDto>>, ApiError> {
    session
        .require(&iam, Permission::SystemNotificationsRead)
        .await?;
    let channels = notifications::channels(&state).await?;
    Ok(Json(channels.into_iter().map(channel_dto).collect()))
}

/// Crée un canal. L'URL et le secret sont chiffrés, jamais renvoyés. Permission :
/// `system.notifications.manage`.
#[utoipa::path(post, path = "/api/admin/v1/settings/notifications/channels", tag = "admin-notifications",
    security(("bearer" = [])), request_body = ChannelRequest,
    responses((status = 201, body = ChannelDto), (status = 403, body = Problem),
        (status = 422, description = "Nom, type, langue ou URL invalide", body = Problem)))]
pub async fn create_channel(
    State(iam): State<IamState>,
    State(state): State<NotificationsState>,
    session: AdminSession,
    Json(request): Json<ChannelRequest>,
) -> Result<(StatusCode, Json<ChannelDto>), ApiError> {
    session
        .require(&iam, Permission::SystemNotificationsManage)
        .await?;
    let view = notifications::create_channel(&state, &session.context, &request.into()).await?;
    Ok((StatusCode::CREATED, Json(channel_dto(view))))
}

/// Modifie un canal ; `url` et `secret` absents gardent les valeurs en place. Permission :
/// `system.notifications.manage`.
#[utoipa::path(put, path = "/api/admin/v1/settings/notifications/channels/{id}", tag = "admin-notifications",
    security(("bearer" = [])), params(("id" = Uuid, Path)), request_body = ChannelRequest,
    responses((status = 200, body = ChannelDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 422, body = Problem)))]
pub async fn update_channel(
    State(iam): State<IamState>,
    State(state): State<NotificationsState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Json(request): Json<ChannelRequest>,
) -> Result<Json<ChannelDto>, ApiError> {
    session
        .require(&iam, Permission::SystemNotificationsManage)
        .await?;
    let view = notifications::update_channel(&state, &session.context, id, &request.into()).await?;
    Ok(Json(channel_dto(view)))
}

/// Supprime un canal ; les règles qui ne visaient que lui disparaissent, les autres l'oublient.
/// Permission : `system.notifications.manage`.
#[utoipa::path(delete, path = "/api/admin/v1/settings/notifications/channels/{id}", tag = "admin-notifications",
    security(("bearer" = [])), params(("id" = Uuid, Path)),
    responses((status = 204), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn delete_channel(
    State(iam): State<IamState>,
    State(state): State<NotificationsState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    session
        .require(&iam, Permission::SystemNotificationsManage)
        .await?;
    notifications::delete_channel(&state, &session.context, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Envoie un événement `test` sur le canal, tout de suite. Permission :
/// `system.notifications.manage`.
#[utoipa::path(post, path = "/api/admin/v1/settings/notifications/channels/{id}/test", tag = "admin-notifications",
    security(("bearer" = [])), params(("id" = Uuid, Path)),
    responses((status = 200, body = TestDto), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn test_channel(
    State(iam): State<IamState>,
    State(state): State<NotificationsState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<Json<TestDto>, ApiError> {
    session
        .require(&iam, Permission::SystemNotificationsManage)
        .await?;
    let outcome = notifications::test_channel(&state, &session.context, id).await?;
    Ok(Json(TestDto {
        ok: outcome.ok,
        status: outcome.status,
        error: outcome.error,
    }))
}

/// Règles configurées. Permission : `system.notifications.read`.
#[utoipa::path(get, path = "/api/admin/v1/settings/notifications/rules", tag = "admin-notifications",
    security(("bearer" = [])), responses((status = 200, body = [RuleDto]), (status = 403, body = Problem)))]
pub async fn list_rules(
    State(iam): State<IamState>,
    State(state): State<NotificationsState>,
    session: AdminSession,
) -> Result<Json<Vec<RuleDto>>, ApiError> {
    session
        .require(&iam, Permission::SystemNotificationsRead)
        .await?;
    let rules = notifications::rules(&state).await?;
    Ok(Json(rules.into_iter().map(rule_dto).collect()))
}

/// Crée une règle. Permission : `system.notifications.manage`.
#[utoipa::path(post, path = "/api/admin/v1/settings/notifications/rules", tag = "admin-notifications",
    security(("bearer" = [])), request_body = RuleRequest,
    responses((status = 201, body = RuleDto), (status = 403, body = Problem),
        (status = 422, description = "Nom, portée, événements ou canaux invalides", body = Problem)))]
pub async fn create_rule(
    State(iam): State<IamState>,
    State(state): State<NotificationsState>,
    session: AdminSession,
    Json(request): Json<RuleRequest>,
) -> Result<(StatusCode, Json<RuleDto>), ApiError> {
    session
        .require(&iam, Permission::SystemNotificationsManage)
        .await?;
    let record = notifications::create_rule(&state, &session.context, &request.into()).await?;
    Ok((StatusCode::CREATED, Json(rule_dto(record))))
}

/// Remplace une règle. Permission : `system.notifications.manage`.
#[utoipa::path(put, path = "/api/admin/v1/settings/notifications/rules/{id}", tag = "admin-notifications",
    security(("bearer" = [])), params(("id" = Uuid, Path)), request_body = RuleRequest,
    responses((status = 200, body = RuleDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 422, body = Problem)))]
pub async fn update_rule(
    State(iam): State<IamState>,
    State(state): State<NotificationsState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Json(request): Json<RuleRequest>,
) -> Result<Json<RuleDto>, ApiError> {
    session
        .require(&iam, Permission::SystemNotificationsManage)
        .await?;
    let record = notifications::update_rule(&state, &session.context, id, &request.into()).await?;
    Ok(Json(rule_dto(record)))
}

/// Supprime une règle. Permission : `system.notifications.manage`.
#[utoipa::path(delete, path = "/api/admin/v1/settings/notifications/rules/{id}", tag = "admin-notifications",
    security(("bearer" = [])), params(("id" = Uuid, Path)),
    responses((status = 204), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn delete_rule(
    State(iam): State<IamState>,
    State(state): State<NotificationsState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    session
        .require(&iam, Permission::SystemNotificationsManage)
        .await?;
    notifications::delete_rule(&state, &session.context, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Journal des envois, les plus récents d'abord. Permission : `system.notifications.read`.
#[utoipa::path(get, path = "/api/admin/v1/settings/notifications/deliveries", tag = "admin-notifications",
    security(("bearer" = [])), params(DeliveriesQuery),
    responses((status = 200, body = [DeliveryDto]), (status = 403, body = Problem)))]
pub async fn list_deliveries(
    State(iam): State<IamState>,
    State(state): State<NotificationsState>,
    session: AdminSession,
    Query(query): Query<DeliveriesQuery>,
) -> Result<Json<Vec<DeliveryDto>>, ApiError> {
    session
        .require(&iam, Permission::SystemNotificationsRead)
        .await?;
    let deliveries = notifications::deliveries(&state, query.limit.unwrap_or(50)).await?;
    Ok(Json(deliveries.into_iter().map(delivery_dto).collect()))
}

/// Abonnement d'un serveur : sa règle et sa destination.
#[derive(Debug, Serialize, ToSchema)]
pub struct SubscriptionDto {
    pub id: Uuid,
    pub name: String,
    /// Événements du serveur (`server.crashed`…).
    pub events: Vec<String>,
    pub enabled: bool,
    /// Destination : canal propre au serveur (`owned`) ou canal partagé des paramètres.
    pub channel: ChannelDto,
    pub owned: bool,
}

fn subscription_dto(view: SubscriptionView) -> SubscriptionDto {
    SubscriptionDto {
        id: view.rule.id,
        name: view.rule.name,
        events: view.rule.events,
        enabled: view.rule.enabled,
        channel: channel_dto(view.channel),
        owned: view.owned,
    }
}

/// Saisie d'un abonnement : `channel_id` d'un canal partagé (demande aussi
/// `system.notifications.read`), ou une destination propre (`kind`, `url`, `secret`,
/// `language`). En modification, `url` et `secret` absents gardent les valeurs en place.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SubscriptionRequest {
    pub name: String,
    pub events: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub channel_id: Option<Uuid>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub secret: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
}

impl From<SubscriptionRequest> for SubscriptionInput {
    fn from(request: SubscriptionRequest) -> Self {
        Self {
            name: request.name,
            events: request.events,
            enabled: request.enabled,
            channel_id: request.channel_id,
            kind: request.kind,
            url: request.url,
            secret: request.secret,
            language: request.language,
        }
    }
}

/// Abonnements d'un serveur et les événements qu'ils peuvent viser. Permission :
/// `servers.notifications.read`.
#[utoipa::path(get, path = "/api/admin/v1/servers/{id}/notifications", tag = "admin-notifications",
    security(("bearer" = [])), params(("id" = Uuid, Path)),
    responses((status = 200, body = SubscriptionsDto), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn list_subscriptions(
    State(iam): State<IamState>,
    State(state): State<NotificationsState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<Json<SubscriptionsDto>, ApiError> {
    session
        .require(&iam, Permission::ServersNotificationsRead)
        .await?;
    let subscriptions = subscriptions::list(&state, id).await?;
    Ok(Json(SubscriptionsDto {
        events: SERVER_EVENTS.into_iter().map(EventKind::as_str).collect(),
        shared_allowed: session
            .principal
            .permissions
            .contains(Permission::SystemNotificationsRead),
        subscriptions: subscriptions.into_iter().map(subscription_dto).collect(),
    }))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SubscriptionsDto {
    /// Événements qu'un abonnement peut viser.
    pub events: Vec<&'static str>,
    /// L'appelant peut viser un canal partagé des paramètres.
    pub shared_allowed: bool,
    pub subscriptions: Vec<SubscriptionDto>,
}

fn may_use_shared(session: &AdminSession) -> bool {
    session
        .principal
        .permissions
        .contains(Permission::SystemNotificationsRead)
}

/// Crée un abonnement. Permission : `servers.notifications.manage`.
#[utoipa::path(post, path = "/api/admin/v1/servers/{id}/notifications", tag = "admin-notifications",
    security(("bearer" = [])), params(("id" = Uuid, Path)), request_body = SubscriptionRequest,
    responses((status = 201, body = SubscriptionDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 422, body = Problem)))]
pub async fn create_subscription(
    State(iam): State<IamState>,
    State(state): State<NotificationsState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Json(request): Json<SubscriptionRequest>,
) -> Result<(StatusCode, Json<SubscriptionDto>), ApiError> {
    session
        .require(&iam, Permission::ServersNotificationsManage)
        .await?;
    let shared = may_use_shared(&session);
    let view = subscriptions::create(&state, &session.context, id, &request.into(), shared).await?;
    Ok((StatusCode::CREATED, Json(subscription_dto(view))))
}

/// Remplace un abonnement. Permission : `servers.notifications.manage`.
#[utoipa::path(put, path = "/api/admin/v1/servers/{id}/notifications/{sub}", tag = "admin-notifications",
    security(("bearer" = [])), params(("id" = Uuid, Path), ("sub" = Uuid, Path)), request_body = SubscriptionRequest,
    responses((status = 200, body = SubscriptionDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 422, body = Problem)))]
pub async fn update_subscription(
    State(iam): State<IamState>,
    State(state): State<NotificationsState>,
    session: AdminSession,
    Path((id, sub)): Path<(Uuid, Uuid)>,
    Json(request): Json<SubscriptionRequest>,
) -> Result<Json<SubscriptionDto>, ApiError> {
    session
        .require(&iam, Permission::ServersNotificationsManage)
        .await?;
    let shared = may_use_shared(&session);
    let view =
        subscriptions::update(&state, &session.context, id, sub, &request.into(), shared).await?;
    Ok(Json(subscription_dto(view)))
}

/// Supprime un abonnement (et sa destination si elle était propre au serveur). Permission :
/// `servers.notifications.manage`.
#[utoipa::path(delete, path = "/api/admin/v1/servers/{id}/notifications/{sub}", tag = "admin-notifications",
    security(("bearer" = [])), params(("id" = Uuid, Path), ("sub" = Uuid, Path)),
    responses((status = 204), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn delete_subscription(
    State(iam): State<IamState>,
    State(state): State<NotificationsState>,
    session: AdminSession,
    Path((id, sub)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    session
        .require(&iam, Permission::ServersNotificationsManage)
        .await?;
    subscriptions::delete(&state, &session.context, id, sub).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Envoie un événement `test` sur la destination de l'abonnement. Permission :
/// `servers.notifications.manage`.
#[utoipa::path(post, path = "/api/admin/v1/servers/{id}/notifications/{sub}/test", tag = "admin-notifications",
    security(("bearer" = [])), params(("id" = Uuid, Path), ("sub" = Uuid, Path)),
    responses((status = 200, body = TestDto), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn test_subscription(
    State(iam): State<IamState>,
    State(state): State<NotificationsState>,
    session: AdminSession,
    Path((id, sub)): Path<(Uuid, Uuid)>,
) -> Result<Json<TestDto>, ApiError> {
    session
        .require(&iam, Permission::ServersNotificationsManage)
        .await?;
    let outcome = subscriptions::test(&state, &session.context, id, sub).await?;
    Ok(Json(TestDto {
        ok: outcome.ok,
        status: outcome.status,
        error: outcome.error,
    }))
}

/// Journal des envois concernant ce serveur. Permission : `servers.notifications.read`.
#[utoipa::path(get, path = "/api/admin/v1/servers/{id}/notifications/deliveries", tag = "admin-notifications",
    security(("bearer" = [])), params(("id" = Uuid, Path), DeliveriesQuery),
    responses((status = 200, body = [DeliveryDto]), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn server_deliveries(
    State(iam): State<IamState>,
    State(state): State<NotificationsState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Query(query): Query<DeliveriesQuery>,
) -> Result<Json<Vec<DeliveryDto>>, ApiError> {
    session
        .require(&iam, Permission::ServersNotificationsRead)
        .await?;
    let deliveries = subscriptions::deliveries(&state, id, query.limit.unwrap_or(50)).await?;
    Ok(Json(deliveries.into_iter().map(delivery_dto).collect()))
}
