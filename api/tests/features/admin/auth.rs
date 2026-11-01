//! Connexion par mot de passe, jetons, rafraîchissement, déconnexion.

use axum::http::{Method, StatusCode};
use kuro_api::iam::domain::Permission;
use serde_json::json;

use crate::support::admin::{PASSWORD, REFRESH_COOKIE, TestAdmin};
use crate::support::http::PANEL_ORIGIN;

#[tokio::test]
async fn login_opens_a_session_with_a_hardened_refresh_cookie() {
    let admin = TestAdmin::start().await;
    let account = admin.account("alice@example.com", &[]).await;

    let response = admin.login_with("Alice@Example.com ", PASSWORD).await;

    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.json["status"], "authenticated");
    assert_eq!(response.json["token_type"], "Bearer");
    let cookie = response.set_cookie_header(REFRESH_COOKIE).unwrap();
    // Préfixe `__Host-` : `Secure`, chemin `/`, sans `Domain` (un autre sous-domaine ne peut pas
    // poser un cookie homonyme).
    let attributes: Vec<&str> = cookie.split("; ").skip(1).collect();
    for attribute in ["HttpOnly", "Secure", "SameSite=Strict", "Path=/"] {
        assert!(
            attributes.contains(&attribute),
            "{attribute} missing in {cookie}"
        );
    }
    assert!(
        !attributes.iter().any(|a| a.starts_with("Domain=")),
        "Domain forbidden by __Host-: {cookie}"
    );
    let event = admin.last_audit("auth.login.succeeded").await;
    assert_eq!(event.actor_id, Some(account.id));
}

#[tokio::test]
async fn wrong_password_and_unknown_account_get_the_same_answer() {
    let admin = TestAdmin::start().await;
    admin.account("alice@example.com", &[]).await;

    let wrong_password = admin
        .login_with("alice@example.com", "not the password")
        .await;
    let unknown = admin.login_with("bob@example.com", PASSWORD).await;

    assert_eq!(wrong_password.status, StatusCode::UNAUTHORIZED);
    assert_eq!(unknown.status, StatusCode::UNAUTHORIZED);
    assert_eq!(wrong_password.json, unknown.json);
    let failures = admin
        .audit()
        .await
        .into_iter()
        .filter(|row| row.action == "auth.login.failed" && row.outcome == "failure")
        .count();
    assert_eq!(failures, 2);
}

#[tokio::test]
async fn account_locks_after_five_failures() {
    let admin = TestAdmin::start().await;
    admin.account("alice@example.com", &[]).await;

    for _ in 0..5 {
        admin
            .login_with("alice@example.com", "not the password")
            .await;
    }
    let with_right_password = admin.login_with("alice@example.com", PASSWORD).await;

    assert_eq!(with_right_password.status, StatusCode::UNAUTHORIZED);
    assert_eq!(with_right_password.json["code"], "invalid_credentials");
    admin.last_audit("auth.login.locked").await;
    assert_eq!(
        admin.last_audit("auth.login.failed").await.details["reason"],
        "locked"
    );
}

#[tokio::test]
async fn missing_or_forged_tokens_are_rejected() {
    let admin = TestAdmin::start().await;
    let account = admin.account("alice@example.com", &[]).await;
    let session = admin.login(&account).await;
    let forged = format!("{}x", session.access_token);

    for token in [None, Some("not-a-jwt"), Some(forged.as_str())] {
        let response = admin.call(Method::GET, "/me", token, None).await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn refresh_rotates_the_token() {
    let admin = TestAdmin::start().await;
    let account = admin.account("alice@example.com", &[]).await;
    let session = admin.login(&account).await;

    let refreshed = admin
        .call_with_cookie(Method::POST, "/auth/refresh", &session.refresh_cookie)
        .await;

    assert_eq!(refreshed.status, StatusCode::OK);
    let new_cookie = refreshed.cookie(REFRESH_COOKIE).unwrap();
    assert_ne!(new_cookie, session.refresh_cookie);
    let token = refreshed.json["access_token"].as_str().unwrap();
    assert_eq!(
        admin
            .call(Method::GET, "/me", Some(token), None)
            .await
            .status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn replaying_a_rotated_refresh_token_revokes_the_whole_session() {
    let admin = TestAdmin::start().await;
    let account = admin.account("alice@example.com", &[]).await;
    let session = admin.login(&account).await;
    let refreshed = admin
        .call_with_cookie(Method::POST, "/auth/refresh", &session.refresh_cookie)
        .await;
    let new_cookie = refreshed.cookie(REFRESH_COOKIE).unwrap();
    let new_token = refreshed.json["access_token"].as_str().unwrap().to_owned();
    admin.expire_rotation_grace().await;

    let replay = admin
        .call_with_cookie(Method::POST, "/auth/refresh", &session.refresh_cookie)
        .await;

    assert_eq!(replay.status, StatusCode::UNAUTHORIZED);
    let legit = admin
        .call_with_cookie(Method::POST, "/auth/refresh", &new_cookie)
        .await;
    assert_eq!(legit.status, StatusCode::UNAUTHORIZED);
    let me = admin.call(Method::GET, "/me", Some(&new_token), None).await;
    assert_eq!(me.status, StatusCode::UNAUTHORIZED);
    let event = admin.last_audit("auth.session.reuse_detected").await;
    assert_eq!(event.outcome, "failure");
    assert_eq!(event.actor_id, Some(account.id));
}

#[tokio::test]
async fn a_concurrent_refresh_within_the_grace_window_keeps_the_session() {
    // Deux onglets envoient le même cookie : le second arrive juste après la rotation.
    let admin = TestAdmin::start().await;
    let account = admin.account("alice@example.com", &[]).await;
    let session = admin.login(&account).await;
    let first = admin
        .call_with_cookie(Method::POST, "/auth/refresh", &session.refresh_cookie)
        .await;
    let new_cookie = first.cookie(REFRESH_COOKIE).unwrap();

    let second = admin
        .call_with_cookie(Method::POST, "/auth/refresh", &session.refresh_cookie)
        .await;

    assert_eq!(second.status, StatusCode::OK);
    assert_eq!(
        second.cookie(REFRESH_COOKIE).as_deref(),
        Some(new_cookie.as_str()),
        "the same successor, never a third cookie"
    );
    let token = second.json["access_token"].as_str().unwrap();
    assert_eq!(
        admin
            .call(Method::GET, "/me", Some(token), None)
            .await
            .status,
        StatusCode::OK
    );
    let event = admin.last_audit("auth.session.refreshed").await;
    assert_eq!(event.outcome, "success");
    assert_eq!(event.details["grace"], true);
    let next = admin
        .call_with_cookie(Method::POST, "/auth/refresh", &new_cookie)
        .await;
    assert_eq!(next.status, StatusCode::OK);
}

#[tokio::test]
async fn a_refresh_whose_response_was_lost_does_not_end_the_session() {
    // Page rechargée pendant un rafraîchissement : l'API a fait tourner le cookie mais le
    // navigateur n'a jamais reçu le nouveau. Il renvoie l'ancien juste après (grâce), puis plus
    // tard : la session doit survivre, et non passer pour un vol.
    let admin = TestAdmin::start().await;
    let account = admin.account("alice@example.com", &[]).await;
    let session = admin.login(&account).await;
    let lost = admin
        .call_with_cookie(Method::POST, "/auth/refresh", &session.refresh_cookie)
        .await;
    assert_eq!(lost.status, StatusCode::OK);

    let reload = admin
        .call_with_cookie(Method::POST, "/auth/refresh", &session.refresh_cookie)
        .await;
    assert_eq!(reload.status, StatusCode::OK);
    let kept = reload
        .cookie(REFRESH_COOKIE)
        .expect("the browser gets the successor back");
    assert_eq!(Some(kept.clone()), lost.cookie(REFRESH_COOKIE));
    admin.expire_rotation_grace().await;

    let later = admin
        .call_with_cookie(Method::POST, "/auth/refresh", &kept)
        .await;
    assert_eq!(later.status, StatusCode::OK, "{}", later.json);
    let event = admin.last_audit("auth.session.refreshed").await;
    assert_eq!(event.details.get("grace"), None);

    // L'ancien cookie, rejoué hors grâce, reste un vol : la session tombe.
    let replay = admin
        .call_with_cookie(Method::POST, "/auth/refresh", &session.refresh_cookie)
        .await;
    assert_eq!(replay.status, StatusCode::UNAUTHORIZED);
    admin.last_audit("auth.session.reuse_detected").await;
}

#[tokio::test]
async fn the_successor_is_kept_encrypted_and_only_during_the_grace() {
    let admin = TestAdmin::start().await;
    let account = admin.account("alice@example.com", &[]).await;
    let session = admin.login(&account).await;
    let first = admin
        .call_with_cookie(Method::POST, "/auth/refresh", &session.refresh_cookie)
        .await;
    let successor = first.cookie(REFRESH_COOKIE).unwrap();

    let stored: Vec<Option<Vec<u8>>> = sqlx::query_scalar(
        "SELECT successor_secret FROM admin_sessions WHERE successor_secret IS NOT NULL",
    )
    .fetch_all(&admin.pool)
    .await
    .unwrap();
    assert_eq!(stored.len(), 1);
    let sealed = stored[0].clone().unwrap();
    assert!(
        !sealed
            .windows(successor.len())
            .any(|w| w == successor.as_bytes()),
        "never stored in clear"
    );

    admin.expire_rotation_grace().await;
    let next = admin
        .call_with_cookie(Method::POST, "/auth/refresh", &successor)
        .await;
    assert_eq!(next.status, StatusCode::OK);
    let left: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM admin_sessions WHERE successor_secret IS NOT NULL",
    )
    .fetch_one(&admin.pool)
    .await
    .unwrap();
    // Seul le successeur de la dernière rotation, encore dans sa grâce, reste.
    assert_eq!(left, 1);
}

#[tokio::test]
async fn the_grace_window_never_revives_a_closed_session() {
    let admin = TestAdmin::start().await;
    let account = admin.account("alice@example.com", &[]).await;
    let session = admin.login(&account).await;
    let refreshed = admin
        .call_with_cookie(Method::POST, "/auth/refresh", &session.refresh_cookie)
        .await;
    let token = refreshed.json["access_token"].as_str().unwrap().to_owned();
    admin
        .call(Method::POST, "/auth/logout", Some(&token), None)
        .await;

    let replay = admin
        .call_with_cookie(Method::POST, "/auth/refresh", &session.refresh_cookie)
        .await;

    assert_eq!(replay.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn logout_invalidates_tokens_immediately() {
    let admin = TestAdmin::start().await;
    let account = admin.account("alice@example.com", &[]).await;
    let session = admin.login(&account).await;

    let logout = admin
        .call(
            Method::POST,
            "/auth/logout",
            Some(&session.access_token),
            None,
        )
        .await;

    assert_eq!(logout.status, StatusCode::NO_CONTENT);
    let me = admin
        .call(Method::GET, "/me", Some(&session.access_token), None)
        .await;
    assert_eq!(me.status, StatusCode::UNAUTHORIZED);
    let refresh = admin
        .call_with_cookie(Method::POST, "/auth/refresh", &session.refresh_cookie)
        .await;
    assert_eq!(refresh.status, StatusCode::UNAUTHORIZED);
    admin.last_audit("auth.logout").await;
}

#[tokio::test]
async fn me_returns_profile_and_effective_permissions() {
    let admin = TestAdmin::start().await;
    let account = admin
        .account(
            "alice@example.com",
            &[Permission::AuditRead, Permission::IamUsersRead],
        )
        .await;
    let session = admin.login(&account).await;

    let me = admin
        .call(Method::GET, "/me", Some(&session.access_token), None)
        .await;

    assert_eq!(me.status, StatusCode::OK);
    assert_eq!(me.json["user"]["email"], "alice@example.com");
    assert_eq!(me.json["all_permissions"], false);
    let mut permissions: Vec<&str> = me.json["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap())
        .collect();
    permissions.sort_unstable();
    assert_eq!(permissions, ["audit.read", "iam.users.read"]);
}

#[tokio::test]
async fn super_admin_sees_every_permission() {
    let admin = TestAdmin::start().await;
    let account = admin.super_admin("root@example.com").await;
    let session = admin.login(&account).await;

    let me = admin
        .call(Method::GET, "/me", Some(&session.access_token), None)
        .await;

    assert_eq!(me.json["all_permissions"], true);
    // Hors permissions d'extensions enregistrées par d'autres tests du binaire (ADR-0051).
    let kuro = me.json["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|key| !crate::support::is_extension_permission(key.as_str().unwrap()))
        .count();
    assert_eq!(kuro, Permission::ALL.len());
}

#[tokio::test]
async fn a_session_older_than_its_maximum_age_cannot_be_refreshed() {
    let admin = TestAdmin::start().await;
    let account = admin.account("alice@example.com", &[]).await;
    let session = admin.login(&account).await;
    // La famille de jetons est née il y a plus longtemps que la durée maximale (30 jours).
    sqlx::query(
        "UPDATE admin_sessions SET created_at = now() - interval '31 days' WHERE user_id = $1",
    )
    .bind(account.id)
    .execute(&admin.pool)
    .await
    .unwrap();

    let refreshed = admin
        .call_with_cookie(Method::POST, "/auth/refresh", &session.refresh_cookie)
        .await;

    assert_eq!(
        refreshed.status,
        StatusCode::UNAUTHORIZED,
        "{}",
        refreshed.json
    );
    let refused = admin.last_audit("auth.session.refreshed").await;
    assert_eq!(refused.outcome, "failure");
    assert_eq!(refused.details["reason"], "max_age");
}
#[tokio::test]
async fn a_refresh_cookie_without_the_host_prefix_is_ignored() {
    // Côté navigateur, `__Host-` empêche un sous-domaine (site web hébergé, ADR-0038) de poser
    // un cookie homonyme ; côté API, seul le nom préfixé est lu : un cookie sous l'ancien nom,
    // ou un nom voisin, n'ouvre rien.
    let admin = TestAdmin::start().await;
    let account = admin.account("alice@example.com", &[]).await;
    let session = admin.login(&account).await;

    let cookie = format!("kuro_admin_refresh={}", session.refresh_cookie);
    let response = admin
        .call_with_headers(Method::POST, "/auth/refresh", &[("cookie", &cookie)], None)
        .await;
    assert_eq!(response.status, StatusCode::UNAUTHORIZED);

    // Le vrai cookie, lui, fonctionne toujours : rien n'a été consommé.
    let refreshed = admin
        .call_with_cookie(Method::POST, "/auth/refresh", &session.refresh_cookie)
        .await;
    assert_eq!(refreshed.status, StatusCode::OK);
}

#[tokio::test]
async fn auth_routes_refuse_a_foreign_origin() {
    // Un site sous le même domaine que le panel est « même site » : le navigateur joint le
    // cookie malgré `SameSite=Strict`. L'en-tête `Origin`, qu'il ne peut pas falsifier, le
    // trahit : ni connexion, ni rotation, ni déconnexion depuis une autre origine.
    let admin = TestAdmin::start().await;
    let account = admin.account("alice@example.com", &[]).await;
    let session = admin.login(&account).await;

    let login = admin
        .call_with_headers(
            Method::POST,
            "/auth/login",
            &[("origin", "https://evil.example.com")],
            Some(json!({ "email": account.email, "password": PASSWORD })),
        )
        .await;
    assert_eq!(login.status, StatusCode::FORBIDDEN);
    assert_eq!(login.json["code"], "forbidden");
    let denied = admin.last_audit("access.denied").await;
    assert_eq!(denied.details["reason"], "origin");
    assert_eq!(denied.details["origin"], "https://evil.example.com");

    let cookie = format!("{REFRESH_COOKIE}={}", session.refresh_cookie);
    let refresh = admin
        .call_with_headers(
            Method::POST,
            "/auth/refresh",
            &[("origin", "https://evil.example.com"), ("cookie", &cookie)],
            None,
        )
        .await;
    assert_eq!(refresh.status, StatusCode::FORBIDDEN);
    assert!(
        refresh.cookie(REFRESH_COOKIE).is_none(),
        "no cookie rotated"
    );

    let bearer = format!("Bearer {}", session.access_token);
    let logout = admin
        .call_with_headers(
            Method::POST,
            "/auth/logout",
            &[
                ("origin", "https://evil.example.com"),
                ("authorization", &bearer),
            ],
            None,
        )
        .await;
    assert_eq!(logout.status, StatusCode::FORBIDDEN);

    // La session est intacte : le cookie tourne normalement depuis le panel.
    let refreshed = admin
        .call_with_headers(
            Method::POST,
            "/auth/refresh",
            &[("origin", PANEL_ORIGIN), ("cookie", &cookie)],
            None,
        )
        .await;
    assert_eq!(refreshed.status, StatusCode::OK);
    assert!(refreshed.cookie(REFRESH_COOKIE).is_some());
}

#[tokio::test]
async fn auth_routes_accept_the_panel_and_clients_without_origin() {
    let admin = TestAdmin::start().await;
    let account = admin.account("alice@example.com", &[]).await;
    let credentials = json!({ "email": account.email, "password": PASSWORD });

    // Origine déclarée dans `http.allowed_origins`.
    let declared = admin
        .call_with_headers(
            Method::POST,
            "/auth/login",
            &[("origin", PANEL_ORIGIN)],
            Some(credentials.clone()),
        )
        .await;
    assert_eq!(declared.status, StatusCode::OK);

    // Même hôte que l'API, en `https://`.
    let same_host = admin
        .call_with_headers(
            Method::POST,
            "/auth/login",
            &[
                ("origin", "https://api.example.com"),
                ("host", "api.example.com"),
            ],
            Some(credentials.clone()),
        )
        .await;
    assert_eq!(same_host.status, StatusCode::OK);

    // Sans `Origin` : client hors navigateur (CLI, script), accepté.
    let script = admin
        .call_with_headers(Method::POST, "/auth/login", &[], Some(credentials))
        .await;
    assert_eq!(script.status, StatusCode::OK);
}
