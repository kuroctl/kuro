//! E-mail (ADR-0046) : réglages SMTP chiffrés et testés, canal e-mail des notifications,
//! invitation d'un admin par lien à usage unique.

use axum::http::{Method, StatusCode};
use kuro_api::notifications::application as notifications;
use serde_json::{Value, json};

use crate::support::admin::TestAdmin;

fn smtp(password: Option<&str>) -> Value {
    let mut body = json!({
        "host": "smtp.example.com", "port": 587, "security": "starttls",
        "username": "kuro", "from_address": "kuro@example.com",
    });
    if let Some(password) = password {
        body["password"] = json!(password);
    }
    body
}

async fn root(admin: &TestAdmin) -> String {
    let root = admin.super_admin("root@example.com").await;
    admin.login(&root).await.access_token
}

#[tokio::test]
async fn smtp_settings_are_stored_encrypted_tested_and_removed() {
    let admin = TestAdmin::start().await;
    let token = root(&admin).await;

    let before = admin
        .call(Method::GET, "/settings/mail", Some(&token), None)
        .await;
    assert_eq!(before.json["configured"], false);
    // Sans `mail.allow_insecure`, le clair est refusé (vérifié plus bas avec `security: none`).
    assert_eq!(before.json["insecure_allowed"], false);
    let refused = admin
        .call(Method::POST, "/settings/mail/test", Some(&token), None)
        .await;
    assert_eq!(refused.status, StatusCode::NOT_FOUND);
    assert_eq!(refused.json["code"], "mail_not_configured");

    let saved = admin
        .call(
            Method::PUT,
            "/settings/mail",
            Some(&token),
            Some(smtp(Some("s3cret"))),
        )
        .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.json);
    assert_eq!(saved.json["configured"], true);
    assert_eq!(saved.json["has_password"], true);
    assert!(!saved.body.contains("s3cret"));
    let event = admin.last_audit("system.mail_updated").await;
    assert_eq!(event.details["host"], "smtp.example.com");
    assert!(!event.details.to_string().contains("s3cret"));

    // Le test part à l'adresse de l'admin, avec le serveur déchiffré et le nom du projet.
    let tested = admin
        .call(Method::POST, "/settings/mail/test", Some(&token), None)
        .await;
    assert_eq!(tested.status, StatusCode::OK, "{}", tested.json);
    assert_eq!(tested.json["to"], "root@example.com");
    let transport = admin.mailer.last_transport().unwrap();
    assert_eq!(transport.password.as_deref(), Some("s3cret"));
    assert_eq!(transport.from_name, "Kuro");
    let sent = admin.mailer.sent();
    assert_eq!(sent.last().unwrap().subject, "[Kuro] Test");

    // Le serveur refuse : cause rendue, sans secret, et auditée en échec.
    admin.mailer.fail("smtp: 535 authentication failed");
    let failed = admin
        .call(Method::POST, "/settings/mail/test", Some(&token), None)
        .await;
    assert_eq!(failed.status, StatusCode::BAD_GATEWAY, "{}", failed.json);
    assert_eq!(failed.json["code"], "mail_send_failed");
    assert!(
        failed.json["params"]["error"]
            .as_str()
            .unwrap()
            .contains("535")
    );
    let event = admin.last_audit("system.mail_tested").await;
    assert_eq!(event.outcome, "failure");
    admin.mailer.succeed();

    // Sans mot de passe : celui en place est gardé ; vide : retiré.
    let kept = admin
        .call(
            Method::PUT,
            "/settings/mail",
            Some(&token),
            Some(smtp(None)),
        )
        .await;
    assert_eq!(kept.json["has_password"], true, "{}", kept.json);
    let cleared = admin
        .call(
            Method::PUT,
            "/settings/mail",
            Some(&token),
            Some(smtp(Some(""))),
        )
        .await;
    assert_eq!(cleared.json["has_password"], false, "{}", cleared.json);

    for (patch, field) in [
        (json!({ "host": "smtp example" }), "host"),
        (json!({ "security": "none" }), "security"),
        (json!({ "from_address": "not-an-address" }), "from_address"),
        (json!({ "port": 0 }), "port"),
    ] {
        let mut body = smtp(None);
        for (key, value) in patch.as_object().unwrap() {
            body[key] = value.clone();
        }
        let response = admin
            .call(Method::PUT, "/settings/mail", Some(&token), Some(body))
            .await;
        assert_eq!(
            response.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{field}: {}",
            response.json
        );
        assert_eq!(
            response.json["fields"][0]["field"], field,
            "{}",
            response.json
        );
    }

    let removed = admin
        .call(Method::DELETE, "/settings/mail", Some(&token), None)
        .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    assert_eq!(
        admin
            .call(Method::GET, "/settings/mail", Some(&token), None)
            .await
            .json["configured"],
        false
    );
    admin.last_audit("system.mail_removed").await;
}

#[tokio::test]
async fn an_email_channel_delivers_events_through_the_project_smtp() {
    let admin = TestAdmin::start().await;
    let token = root(&admin).await;
    admin
        .call(
            Method::PATCH,
            "/settings",
            Some(&token),
            Some(json!({ "project_name": "Studio Nuage" })),
        )
        .await;

    // Destinataires : adresses valides, de une à vingt.
    let too_many: Vec<String> = (0..21).map(|i| format!("a{i}@example.com")).collect();
    for recipients in [json!([]), json!(["nope"]), json!(too_many)] {
        let refused = admin
            .call(
                Method::POST,
                "/settings/notifications/channels",
                Some(&token),
                Some(json!({ "name": "Ops", "kind": "email", "recipients": recipients, "language": "fr" })),
            )
            .await;
        assert_eq!(
            refused.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{}",
            refused.json
        );
        assert_eq!(refused.json["fields"][0]["field"], "recipients");
    }
    let created = admin
        .call(
            Method::POST,
            "/settings/notifications/channels",
            Some(&token),
            Some(json!({
                "name": "Ops", "kind": "email", "language": "fr",
                "recipients": ["ops@example.com", "Ops@Example.com", "boss@example.com"],
            })),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json);
    assert_eq!(created.json["kind"], "email");
    assert_eq!(created.json["url_hint"], "o…@example.com (+1)");
    assert!(!created.body.contains("boss@example.com"));
    let channel_id = created.json["id"].as_str().unwrap().to_owned();

    // Sans SMTP, le test du canal dit pourquoi ; avec, le message part aux destinataires.
    let test = admin
        .call(
            Method::POST,
            &format!("/settings/notifications/channels/{channel_id}/test"),
            Some(&token),
            None,
        )
        .await;
    assert_eq!(test.status, StatusCode::OK, "{}", test.json);
    assert_eq!(test.json["ok"], false);
    assert_eq!(test.json["error"], "mail_not_configured");

    admin
        .call(
            Method::PUT,
            "/settings/mail",
            Some(&token),
            Some(smtp(Some("pw"))),
        )
        .await;
    let test = admin
        .call(
            Method::POST,
            &format!("/settings/notifications/channels/{channel_id}/test"),
            Some(&token),
            None,
        )
        .await;
    assert_eq!(test.json["ok"], true, "{}", test.json);
    let message = admin.mailer.sent().pop().unwrap();
    assert_eq!(message.to, vec!["ops@example.com", "boss@example.com"]);
    assert_eq!(message.subject, "[Studio Nuage] Test de notification");
    assert!(message.html.as_deref().unwrap().contains("Studio Nuage"));

    // Un événement couvert par une règle passe par la file et arrive par e-mail.
    let rule = admin
        .call(
            Method::POST,
            "/settings/notifications/rules",
            Some(&token),
            Some(json!({ "name": "Tout", "scope": "all", "events": ["node.offline"], "channels": [channel_id] })),
        )
        .await;
    assert_eq!(rule.status, StatusCode::CREATED, "{}", rule.json);
    let event = kuro_api::notifications::domain::Event::now(
        kuro_api::notifications::domain::EventKind::NodeOffline,
    )
    .node(kuro_api::notifications::domain::NodeRef {
        id: uuid::Uuid::now_v7(),
        name: "node-1".to_owned(),
    });
    notifications::emit(&admin.pool, event).await.unwrap();
    let attempted = notifications::deliver_due(&admin.notifications)
        .await
        .unwrap();
    assert_eq!(attempted, 1);
    let message = admin.mailer.sent().pop().unwrap();
    assert_eq!(message.subject, "[Studio Nuage] Nœud hors ligne");
    assert!(message.text.contains("node-1"));
    let deliveries = admin
        .call(
            Method::GET,
            "/settings/notifications/deliveries",
            Some(&token),
            None,
        )
        .await;
    assert_eq!(deliveries.json[0]["status"], "sent", "{}", deliveries.json);
}

#[tokio::test]
async fn an_admin_is_invited_by_email_and_accepts_the_link_once() {
    let admin = TestAdmin::start().await;
    let token = root(&admin).await;

    // Sans SMTP : pas d'invitation ; sans mot de passe ni invitation : champ manquant.
    let refused = admin
        .call(
            Method::POST,
            "/users",
            Some(&token),
            Some(json!({ "email": "new@example.com", "display_name": "New", "invite": true })),
        )
        .await;
    assert_eq!(
        refused.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        refused.json
    );
    assert_eq!(refused.json["code"], "mail_not_configured");
    let refused = admin
        .call(
            Method::POST,
            "/users",
            Some(&token),
            Some(json!({ "email": "new@example.com", "display_name": "New" })),
        )
        .await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(refused.json["fields"][0]["field"], "password");

    admin
        .call(
            Method::PUT,
            "/settings/mail",
            Some(&token),
            Some(smtp(Some("pw"))),
        )
        .await;
    let created = admin
        .call(
            Method::POST,
            "/users",
            Some(&token),
            Some(json!({ "email": "new@example.com", "display_name": "New", "invite": true })),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json);
    assert_eq!(created.json["invitation_pending"], true);
    assert_eq!(created.json["password_change_required"], true);
    let id = created.json["id"].as_str().unwrap().to_owned();

    // Le lien est dans l'e-mail, nulle part ailleurs.
    let message = admin.mailer.sent().pop().unwrap();
    assert_eq!(message.to, vec!["new@example.com"]);
    assert_eq!(message.subject, "[Kuro] Invitation");
    let link = message
        .text
        .lines()
        .find(|line| line.starts_with("https://panel.example.com/invitation/"))
        .unwrap()
        .to_owned();
    let invite_token = link.rsplit('/').next().unwrap().to_owned();
    assert!(invite_token.starts_with("kuro_inv_"));
    let event = admin.last_audit("iam.user.invited").await;
    assert_eq!(event.target_id.as_deref(), Some(id.as_str()));
    assert!(
        !admin
            .audit()
            .await
            .iter()
            .any(|row| row.details.to_string().contains("kuro_inv_"))
    );

    // Le compte n'a pas de mot de passe utilisable.
    let login = admin
        .call(
            Method::POST,
            "/auth/login",
            None,
            Some(json!({ "email": "new@example.com", "password": "whatever it is 123" })),
        )
        .await;
    assert_eq!(login.status, StatusCode::UNAUTHORIZED);

    // Acceptation : mot de passe faible refusé, bon jeton accepté, puis connexion directe.
    let weak = admin
        .raw(
            Method::POST,
            "/api/admin/v1/auth/invitation",
            &[("origin", crate::support::http::PANEL_ORIGIN)],
            Some((
                "application/json",
                json!({ "token": invite_token, "password": "short" }).to_string(),
            )),
        )
        .await;
    assert_eq!(
        weak.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        weak.json
    );
    let accepted = admin
        .raw(
            Method::POST,
            "/api/admin/v1/auth/invitation",
            &[("origin", crate::support::http::PANEL_ORIGIN)],
            Some((
                "application/json",
                json!({ "token": invite_token, "password": "my very own password" }).to_string(),
            )),
        )
        .await;
    assert_eq!(accepted.status, StatusCode::OK, "{}", accepted.json);
    assert_eq!(accepted.json["status"], "authenticated");
    let access = accepted.json["access_token"].as_str().unwrap().to_owned();
    let me = admin.call(Method::GET, "/me", Some(&access), None).await;
    assert_eq!(me.json["user"]["email"], "new@example.com");
    assert_eq!(me.json["user"]["password_change_required"], false);
    assert_eq!(me.json["user"]["invitation_pending"], false);
    admin.last_audit("iam.invitation.accepted").await;

    // Une seule fois.
    let again = admin
        .raw(
            Method::POST,
            "/api/admin/v1/auth/invitation",
            &[("origin", crate::support::http::PANEL_ORIGIN)],
            Some((
                "application/json",
                json!({ "token": invite_token, "password": "another fine password" }).to_string(),
            )),
        )
        .await;
    assert_eq!(again.status, StatusCode::FORBIDDEN);
    assert_eq!(again.json["code"], "invalid_token");

    // Réinvitation : nouveau lien ; un mot de passe imposé ensuite le révoque.
    let reinvited = admin
        .call(
            Method::POST,
            &format!("/users/{id}/invite"),
            Some(&token),
            None,
        )
        .await;
    assert_eq!(
        reinvited.status,
        StatusCode::NO_CONTENT,
        "{}",
        reinvited.json
    );
    let message = admin.mailer.sent().pop().unwrap();
    let new_token = message
        .text
        .lines()
        .find(|line| line.starts_with("https://panel.example.com/invitation/"))
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap()
        .to_owned();
    assert_ne!(new_token, invite_token);
    let reset = admin
        .call(
            Method::PUT,
            &format!("/users/{id}/password"),
            Some(&token),
            Some(json!({ "password": "imposed by the admin" })),
        )
        .await;
    assert_eq!(reset.status, StatusCode::NO_CONTENT, "{}", reset.json);
    let revoked = admin
        .raw(
            Method::POST,
            "/api/admin/v1/auth/invitation",
            &[("origin", crate::support::http::PANEL_ORIGIN)],
            Some((
                "application/json",
                json!({ "token": new_token, "password": "another fine password" }).to_string(),
            )),
        )
        .await;
    assert_eq!(revoked.status, StatusCode::FORBIDDEN);

    // Le serveur SMTP refuse : l'invitation n'est pas créée.
    admin.mailer.fail("smtp: 451 try later");
    let failed = admin
        .call(
            Method::POST,
            "/users",
            Some(&token),
            Some(json!({ "email": "other@example.com", "display_name": "Other", "invite": true })),
        )
        .await;
    assert_eq!(
        failed.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        failed.json
    );
    assert_eq!(failed.json["code"], "mail_send_failed");
}

/// Envoi réel par `lettre` vers le Mailpit de `docker-compose.dev.yml` (127.0.0.1:1025, sans
/// chiffrement) : à lancer à la main, `cargo test -p kuro-api --test features -- --ignored mailpit`,
/// puis lire le message sur http://localhost:8025.
#[tokio::test]
#[ignore = "needs the dev Mailpit (docker compose -f docker-compose.dev.yml up -d mailpit)"]
async fn mailpit_receives_a_message_sent_through_lettre() {
    use kuro_api::mail::{LettreMailer, Mailer, Message, Security, SmtpTransport};
    let transport = SmtpTransport {
        host: "127.0.0.1".to_owned(),
        port: 1025,
        security: Security::None,
        username: None,
        password: None,
        from_address: "kuro@example.test".to_owned(),
        from_name: "Kuro dev".to_owned(),
    };
    // Le vrai gabarit du projet, logo compris : à regarder dans Mailpit.
    let email = kuro_api::mail::Email {
        brand: "Kuro dev".to_owned(),
        title: "Invitation".to_owned(),
        paragraphs: vec![
            "Alice,".to_owned(),
            "Vous êtes invité·e à rejoindre Kuro dev. Choisissez votre mot de passe en ouvrant ce lien, valable 48 h et une seule fois.".to_owned(),
        ],
        action: Some((
            "Choisir mon mot de passe".to_owned(),
            "http://localhost:5173/invitation/kuro_inv_demo".to_owned(),
        )),
        note: Some("Envoyé par le test ignoré `mailpit`.".to_owned()),
    };
    let message = Message::from_template(vec!["ops@example.test".to_owned()], &email);
    LettreMailer.send(transport, message).await.unwrap();
}
