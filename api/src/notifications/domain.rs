//! Modèle des notifications (ADR-0044) : événements émis par les cas d'usage, canaux (type,
//! configuration chiffrée), règles (portée, événements, canaux) et règles de validation.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;

/// Liste fermée des événements notifiables. `Test` ne sert qu'au bouton « Tester » d'un canal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    #[serde(rename = "server.crashed")]
    ServerCrashed,
    #[serde(rename = "server.schedule_failed")]
    ServerScheduleFailed,
    #[serde(rename = "server.backup_failed")]
    ServerBackupFailed,
    #[serde(rename = "server.upgrade_failed")]
    ServerUpgradeFailed,
    #[serde(rename = "server.disk_high")]
    ServerDiskHigh,
    #[serde(rename = "node.offline")]
    NodeOffline,
    #[serde(rename = "node.online")]
    NodeOnline,
    #[serde(rename = "test")]
    Test,
}

impl EventKind {
    /// Événements qu'une règle peut viser (tout sauf `test`).
    pub const RULE_EVENTS: [Self; 7] = [
        Self::ServerCrashed,
        Self::ServerScheduleFailed,
        Self::ServerBackupFailed,
        Self::ServerUpgradeFailed,
        Self::ServerDiskHigh,
        Self::NodeOffline,
        Self::NodeOnline,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ServerCrashed => "server.crashed",
            Self::ServerScheduleFailed => "server.schedule_failed",
            Self::ServerBackupFailed => "server.backup_failed",
            Self::ServerUpgradeFailed => "server.upgrade_failed",
            Self::ServerDiskHigh => "server.disk_high",
            Self::NodeOffline => "node.offline",
            Self::NodeOnline => "node.online",
            Self::Test => "test",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::RULE_EVENTS
            .into_iter()
            .chain(std::iter::once(Self::Test))
            .find(|kind| kind.as_str() == text)
    }
}

/// Serveur concerné par un événement : de quoi l'identifier et le nommer, rien de plus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerRef {
    pub id: Uuid,
    pub name: String,
    pub prefab_id: String,
    pub node_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeRef {
    pub id: Uuid,
    pub name: String,
}

/// Événement émis par un cas d'usage. Jamais de secret ni de ligne de console : un `reason`
/// stable, et quelques détails chiffrés (`details`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub kind: EventKind,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<ServerRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<NodeRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub details: Value,
}

impl Event {
    pub fn now(kind: EventKind) -> Self {
        Self {
            kind,
            at: OffsetDateTime::now_utc(),
            server: None,
            node: None,
            reason: None,
            details: Value::Null,
        }
    }

    pub fn server(mut self, server: ServerRef) -> Self {
        self.server = Some(server);
        self
    }

    pub fn node(mut self, node: NodeRef) -> Self {
        self.node = Some(node);
        self
    }

    pub fn reason(mut self, reason: impl Into<String>) -> Self {
        let reason: String = reason.into();
        self.reason = (!reason.is_empty()).then_some(clean(&reason, 500));
        self
    }

    pub fn details(mut self, details: Value) -> Self {
        self.details = details;
        self
    }
}

/// Texte sans caractère de contrôle, borné.
pub fn clean(text: &str, max: usize) -> String {
    text.chars().filter(|c| !c.is_control()).take(max).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    /// `POST` JSON signé HMAC vers une URL quelconque.
    Webhook,
    /// Webhook entrant Discord (embed).
    Discord,
    /// Webhook entrant Slack (blocks).
    Slack,
}

impl ChannelKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Webhook => "webhook",
            Self::Discord => "discord",
            Self::Slack => "slack",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        [Self::Webhook, Self::Discord, Self::Slack]
            .into_iter()
            .find(|kind| kind.as_str() == text)
    }
}

/// Configuration d'un canal, chiffrée en base : l'URL porte souvent un jeton (Discord, Slack).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelConfig {
    pub url: String,
    /// Secret HMAC du webhook générique ; sans secret, pas d'en-tête de signature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
}

/// Portée d'une règle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    All,
    Node(Uuid),
    Server(Uuid),
}

impl Scope {
    pub fn kind(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Node(_) => "node",
            Self::Server(_) => "server",
        }
    }

    /// La règle couvre l'événement : tout, le nœud du serveur ou de l'événement, ou le serveur.
    pub fn covers(self, event: &Event) -> bool {
        match self {
            Self::All => true,
            Self::Node(node) => {
                event.node.as_ref().is_some_and(|n| n.id == node)
                    || event.server.as_ref().is_some_and(|s| s.node_id == node)
            }
            Self::Server(server) => event.server.as_ref().is_some_and(|s| s.id == server),
        }
    }
}

/// URL de webhook acceptable : `https://`, un hôte, pas d'identifiants dans l'URL.
pub fn valid_webhook_url(url: &str) -> bool {
    let Ok(uri) = url.parse::<hyper::Uri>() else {
        return false;
    };
    uri.scheme_str() == Some("https")
        && uri.host().is_some_and(|host| !host.is_empty())
        && !url.contains('@')
        && url.len() <= 2048
        && !url.chars().any(char::is_control)
}

/// URL tronquée pour l'affichage : schéma, hôte et les premiers caractères du chemin ; le reste
/// (jetons Discord, Slack…) n'est jamais rendu.
pub fn url_hint(url: &str) -> String {
    let Ok(uri) = url.parse::<hyper::Uri>() else {
        return String::new();
    };
    let host = uri.authority().map_or("", |a| a.as_str());
    let path = uri.path();
    let shown: String = path.chars().take(12).collect();
    let suffix = if path.len() > 12 { "…" } else { "" };
    format!("https://{host}{shown}{suffix}")
}

/// Délai avant la tentative suivante : 1 min, 5 min, 30 min ; au-delà, abandon.
pub const MAX_ATTEMPTS: i32 = 4;
pub fn backoff(attempts: i32) -> Option<time::Duration> {
    match attempts {
        1 => Some(time::Duration::minutes(1)),
        2 => Some(time::Duration::minutes(5)),
        3 => Some(time::Duration::minutes(30)),
        _ => None,
    }
}

/// Envois par canal et par minute au-delà desquels les suivants attendent la minute d'après.
pub const RATE_LIMIT_PER_MINUTE: i64 = 30;
/// Seuil d'usage disque (part du quota) qui déclenche `server.disk_high`, et seuil de retour.
pub const DISK_HIGH_RATIO: f64 = 0.9;
pub const DISK_RESET_RATIO: f64 = 0.8;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_kinds_round_trip_through_serde_and_text() {
        for kind in EventKind::RULE_EVENTS.into_iter().chain([EventKind::Test]) {
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!("\"{}\"", kind.as_str()));
            assert_eq!(serde_json::from_str::<EventKind>(&json).unwrap(), kind);
            assert_eq!(EventKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(EventKind::parse("server.exploded"), None);
    }

    #[test]
    fn scopes_cover_the_right_events() {
        let node = Uuid::now_v7();
        let other = Uuid::now_v7();
        let server = Uuid::now_v7();
        let event = Event::now(EventKind::ServerCrashed).server(ServerRef {
            id: server,
            name: "Survie".into(),
            prefab_id: "minecraft-java".into(),
            node_id: node,
        });
        assert!(Scope::All.covers(&event));
        assert!(Scope::Node(node).covers(&event));
        assert!(!Scope::Node(other).covers(&event));
        assert!(Scope::Server(server).covers(&event));
        assert!(!Scope::Server(other).covers(&event));
        let node_event = Event::now(EventKind::NodeOffline).node(NodeRef {
            id: node,
            name: "dev-1".into(),
        });
        assert!(Scope::Node(node).covers(&node_event));
        assert!(!Scope::Server(server).covers(&node_event));
    }

    #[test]
    fn webhook_urls_are_https_without_credentials_and_hints_hide_the_path() {
        assert!(valid_webhook_url(
            "https://discord.com/api/webhooks/123/abcDEF"
        ));
        assert!(!valid_webhook_url("http://example.com/hook"));
        assert!(!valid_webhook_url("https://user:pass@example.com/hook"));
        assert!(!valid_webhook_url("not a url"));
        assert_eq!(
            url_hint("https://discord.com/api/webhooks/123456/abcDEFsecret"),
            "https://discord.com/api/webhook…"
        );
        assert_eq!(url_hint("https://hooks.test/h"), "https://hooks.test/h");
    }

    #[test]
    fn backoff_grows_then_stops() {
        assert_eq!(backoff(1), Some(time::Duration::minutes(1)));
        assert_eq!(backoff(3), Some(time::Duration::minutes(30)));
        assert_eq!(backoff(MAX_ATTEMPTS), None);
    }
}
