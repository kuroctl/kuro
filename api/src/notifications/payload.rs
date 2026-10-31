//! Corps des notifications selon le canal : JSON signé pour le webhook générique, embed Discord,
//! blocks Slack. Textes en français ou en anglais selon la langue du canal.

use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use super::domain::{ChannelConfig, ChannelKind, Event, EventKind};
use super::sender::Outgoing;

/// Titre et phrase d'un événement, dans la langue du canal.
pub struct Texts {
    pub title: String,
    pub body: String,
}

pub fn texts(event: &Event, lang: &str) -> Texts {
    let fr = lang != "en";
    let server = event
        .server
        .as_ref()
        .map(|s| s.name.as_str())
        .unwrap_or("?");
    let node = event.node.as_ref().map(|n| n.name.as_str()).unwrap_or("?");
    let reason = event.reason.as_deref();
    let with_reason = |text: String| match reason {
        Some(reason) if fr => format!("{text} Cause : {reason}."),
        Some(reason) => format!("{text} Reason: {reason}."),
        None => text,
    };
    let (title, body) = match event.kind {
        EventKind::ServerCrashed => (
            if fr {
                "Serveur arrêté de façon inattendue"
            } else {
                "Server crashed"
            },
            with_reason(if fr {
                format!("Le serveur « {server} » s'est arrêté de façon inattendue.")
            } else {
                format!("Server \"{server}\" stopped unexpectedly.")
            }),
        ),
        EventKind::ServerScheduleFailed => (
            if fr {
                "Tâche planifiée en échec"
            } else {
                "Scheduled task failed"
            },
            with_reason(if fr {
                format!("Une tâche planifiée du serveur « {server} » a échoué.")
            } else {
                format!("A scheduled task of server \"{server}\" failed.")
            }),
        ),
        EventKind::ServerBackupFailed => (
            if fr {
                "Sauvegarde en échec"
            } else {
                "Backup failed"
            },
            with_reason(if fr {
                format!("La sauvegarde du serveur « {server} » a échoué.")
            } else {
                format!("The backup of server \"{server}\" failed.")
            }),
        ),
        EventKind::ServerUpgradeFailed => (
            if fr {
                "Mise à jour en échec"
            } else {
                "Upgrade failed"
            },
            with_reason(if fr {
                format!("La mise à jour du serveur « {server} » a échoué.")
            } else {
                format!("The upgrade of server \"{server}\" failed.")
            }),
        ),
        EventKind::ServerDiskHigh => {
            let percent = event.details["percent"].as_u64().unwrap_or(90);
            (
                if fr {
                    "Disque presque plein"
                } else {
                    "Disk almost full"
                },
                if fr {
                    format!("Le serveur « {server} » utilise {percent} % de son quota disque.")
                } else {
                    format!("Server \"{server}\" uses {percent}% of its disk quota.")
                },
            )
        }
        EventKind::ServerCpuHigh | EventKind::ServerMemoryHigh => {
            let percent = event.details["percent"].as_u64().unwrap_or(0);
            let minutes = event.details["minutes"].as_u64().unwrap_or(0);
            let cpu = event.kind == EventKind::ServerCpuHigh;
            (
                match (cpu, fr) {
                    (true, true) => "Processeur saturé",
                    (true, false) => "CPU saturated",
                    (false, true) => "Mémoire saturée",
                    (false, false) => "Memory saturated",
                },
                match (cpu, fr) {
                    (true, true) => format!(
                        "Le serveur « {server} » utilise {percent} % de son processeur depuis {minutes} min."
                    ),
                    (true, false) => format!(
                        "Server \"{server}\" has been using {percent}% of its CPU for {minutes} min."
                    ),
                    (false, true) => format!(
                        "Le serveur « {server} » utilise {percent} % de sa mémoire depuis {minutes} min."
                    ),
                    (false, false) => format!(
                        "Server \"{server}\" has been using {percent}% of its memory for {minutes} min."
                    ),
                },
            )
        }
        EventKind::NodeOffline => (
            if fr {
                "Nœud hors ligne"
            } else {
                "Node offline"
            },
            if fr {
                format!("Le nœud « {node} » ne répond plus.")
            } else {
                format!("Node \"{node}\" is no longer responding.")
            },
        ),
        EventKind::NodeOnline => (
            if fr {
                "Nœud de retour"
            } else {
                "Node back online"
            },
            if fr {
                format!("Le nœud « {node} » est de nouveau en ligne.")
            } else {
                format!("Node \"{node}\" is back online.")
            },
        ),
        EventKind::Test => (
            if fr {
                "Test de notification"
            } else {
                "Notification test"
            },
            if fr {
                "Ce canal reçoit bien les notifications de Kuro.".to_owned()
            } else {
                "This channel receives Kuro notifications.".to_owned()
            },
        ),
    };
    Texts {
        title: title.to_owned(),
        body,
    }
}

/// Page du panel à ouvrir depuis la notification, si l'URL du panel est connue.
fn link(event: &Event, panel_url: Option<&str>) -> Option<String> {
    let base = panel_url?.trim_end_matches('/');
    if let Some(server) = &event.server {
        return Some(format!("{base}/servers/{}", server.id));
    }
    if let Some(node) = &event.node {
        return Some(format!("{base}/nodes/{}", node.id));
    }
    Some(base.to_owned())
}

/// Requête à envoyer pour `event` sur un canal de type `kind`.
pub fn build(
    kind: ChannelKind,
    config: &ChannelConfig,
    event: &Event,
    lang: &str,
    delivery_id: Uuid,
    panel_url: Option<&str>,
) -> Outgoing {
    let texts = texts(event, lang);
    let url = link(event, panel_url);
    let at = event.at.format(&Rfc3339).unwrap_or_default();
    let (body, mut headers) = match kind {
        ChannelKind::Webhook => (
            json!({
                "id": delivery_id,
                "type": event.kind.as_str(),
                "at": at,
                "title": texts.title,
                "message": texts.body,
                "server": event.server.as_ref().map(|s| json!({
                    "id": s.id, "name": s.name, "prefab_id": s.prefab_id, "node_id": s.node_id,
                    "url": url,
                })),
                "node": event.node.as_ref().map(|n| json!({ "id": n.id, "name": n.name, "url": url })),
                "reason": event.reason,
                "details": event.details,
            }),
            vec![
                ("X-Kuro-Event".to_owned(), event.kind.as_str().to_owned()),
                ("X-Kuro-Delivery".to_owned(), delivery_id.to_string()),
            ],
        ),
        ChannelKind::Discord => (
            json!({
                "embeds": [{
                    "title": texts.title,
                    "description": texts.body,
                    "url": url,
                    "color": color(event.kind),
                    "timestamp": at,
                    "footer": { "text": "Kuro" },
                }],
            }),
            Vec::new(),
        ),
        // Un canal e-mail passe par `email()` : rien à poster.
        ChannelKind::Email => (json!({}), Vec::new()),
        ChannelKind::Slack => {
            let open = if lang == "en" { "Open" } else { "Ouvrir" };
            let mut text = format!("*{}*\n{}", texts.title, texts.body);
            if let Some(url) = &url {
                text.push_str(&format!("\n<{url}|{open}>"));
            }
            (
                json!({
                    "text": format!("{} — {}", texts.title, texts.body),
                    "blocks": [{ "type": "section", "text": { "type": "mrkdwn", "text": text } }],
                }),
                Vec::new(),
            )
        }
    };
    let body = serde_json::to_vec(&body).unwrap_or_default();
    if kind == ChannelKind::Webhook
        && let Some(secret) = config.secret.as_deref().filter(|s| !s.is_empty())
    {
        headers.push(("X-Kuro-Signature".to_owned(), signature(secret, &body)));
    }
    Outgoing {
        url: config.url.clone(),
        body,
        headers,
    }
}

/// `sha256=<hex>` : HMAC-SHA256 du corps avec le secret du canal.
pub fn signature(secret: &str, body: &[u8]) -> String {
    // HMAC accepte une clé de toute longueur : l'erreur n'arrive pas ; sans signature plutôt
    // qu'une panique.
    let Ok(mut mac) = <Hmac<Sha256> as KeyInit>::new_from_slice(secret.as_bytes()) else {
        return String::new();
    };
    mac.update(body);
    let digest = mac.finalize().into_bytes();
    format!(
        "sha256={}",
        digest
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    )
}

/// Couleur de l'embed Discord : rouge pour un incident, vert pour un retour, gris pour un test.
fn color(kind: EventKind) -> u32 {
    match kind {
        EventKind::NodeOnline => 0x2E_CC71,
        EventKind::Test => 0x95_A5A6,
        EventKind::ServerDiskHigh | EventKind::ServerCpuHigh | EventKind::ServerMemoryHigh => {
            0xF3_9C12
        }
        _ => 0xE7_4C3C,
    }
}

/// Message e-mail d'un événement (ADR-0046) : sujet « [Projet] Titre », corps texte et HTML
/// sobre avec le lien vers le panel.
pub fn email(
    config: &ChannelConfig,
    event: &Event,
    lang: &str,
    brand: &str,
    panel_url: Option<&str>,
) -> crate::mail::Message {
    let texts = texts(event, lang);
    let url = link(event, panel_url);
    let open = if lang == "en" {
        "Open in the panel"
    } else {
        "Ouvrir dans le panel"
    };
    let at = event.at.format(&Rfc3339).unwrap_or_default();
    let email = crate::mail::Email {
        brand: brand.to_owned(),
        title: texts.title,
        paragraphs: vec![texts.body],
        action: url.map(|url| (open.to_owned(), url)),
        note: Some(at),
    };
    crate::mail::Message::from_template(config.recipients.clone(), &email)
}

/// Événement tel que stocké (`jsonb`), relu pour l'envoi.
pub fn event_from(value: &Value) -> Option<Event> {
    serde_json::from_value(value.clone()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notifications::domain::ServerRef;

    fn event() -> Event {
        Event::now(EventKind::ServerCrashed)
            .server(ServerRef {
                id: Uuid::nil(),
                name: "Survie".into(),
                prefab_id: "minecraft-java".into(),
                node_id: Uuid::nil(),
            })
            .reason("exit_code_137")
    }

    #[test]
    fn the_generic_webhook_is_signed_and_links_to_the_panel() {
        let config = ChannelConfig {
            url: "https://hooks.test/kuro".into(),
            secret: Some("s3cret".into()),
            recipients: Vec::new(),
        };
        let outgoing = build(
            ChannelKind::Webhook,
            &config,
            &event(),
            "fr",
            Uuid::nil(),
            Some("https://panel.test/"),
        );
        let body: Value = serde_json::from_slice(&outgoing.body).unwrap();
        assert_eq!(body["type"], "server.crashed");
        assert_eq!(
            body["server"]["url"],
            "https://panel.test/servers/00000000-0000-0000-0000-000000000000"
        );
        assert!(body["message"].as_str().unwrap().contains("Survie"));
        let signature_header = outgoing
            .headers
            .iter()
            .find(|(name, _)| name == "X-Kuro-Signature")
            .map(|(_, value)| value.clone())
            .unwrap();
        assert_eq!(signature_header, signature("s3cret", &outgoing.body));
        assert!(
            outgoing
                .headers
                .iter()
                .any(|(n, v)| n == "X-Kuro-Event" && v == "server.crashed")
        );
    }

    #[test]
    fn discord_and_slack_bodies_carry_the_texts_in_the_channel_language() {
        let config = ChannelConfig {
            url: "https://discord.com/api/webhooks/1/x".into(),
            secret: None,
            recipients: Vec::new(),
        };
        let discord = build(
            ChannelKind::Discord,
            &config,
            &event(),
            "en",
            Uuid::nil(),
            None,
        );
        let body: Value = serde_json::from_slice(&discord.body).unwrap();
        assert_eq!(body["embeds"][0]["title"], "Server crashed");
        assert!(
            body["embeds"][0]["description"]
                .as_str()
                .unwrap()
                .contains("Reason: exit_code_137")
        );
        assert!(discord.headers.is_empty());
        let slack = build(
            ChannelKind::Slack,
            &config,
            &event(),
            "fr",
            Uuid::nil(),
            None,
        );
        let body: Value = serde_json::from_slice(&slack.body).unwrap();
        assert!(
            body["blocks"][0]["text"]["text"]
                .as_str()
                .unwrap()
                .contains("Survie")
        );
    }
}
