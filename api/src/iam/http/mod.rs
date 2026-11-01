//! Routes HTTP de l'espace admin pour le domaine `iam`, montées sous `/api/admin/v1`.

mod auth;
mod error;
mod me;
mod owner;
mod permissions;
mod roles;
mod session;
mod users;

use axum::Router;
use axum::extract::FromRef;
use axum::middleware::from_fn;
use axum::routing::{delete, get, post, put};
use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityScheme};
use utoipa::{Modify, OpenApi};

use crate::http::rate_limit::limit_unauthenticated;
use crate::iam::application::IamState;

pub use owner::OwnerSession;
pub use session::AdminSession;

#[derive(OpenApi)]
#[openapi(
    paths(
        auth::login, auth::change_required_password, auth::verify_totp, auth::start_totp_setup, auth::confirm_totp,
        auth::refresh, auth::logout, auth::accept_invitation, auth::forgot_password,
        auth::reset_password,
        me::get, me::change_password, me::start_totp_setup, me::confirm_totp,
        me::regenerate_recovery_codes, me::disable_totp, me::elevate,
        permissions::catalog,
        roles::list, roles::get, roles::create, roles::update, roles::delete,
        users::list, users::get, users::create, users::update, users::delete,
        users::set_roles, users::reset_password, users::reset_totp, users::revoke_sessions,
        users::invite,
    ),
    tags(
        (name = "admin-auth", description = "Connexion à l'espace admin (mot de passe, TOTP, sessions)"),
        (name = "admin-iam", description = "Comptes admin, rôles et permissions (RBAC)"),
    ),
    modifiers(&BearerAuth)
)]
pub struct IamApi;

/// Schéma de sécurité `bearer` (jeton d'accès de l'espace admin).
struct BearerAuth;

impl Modify for BearerAuth {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer",
            SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
        );
    }
}

pub fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    IamState: FromRef<S>,
    // Vérification d'`Origin` sur les routes d'authentification (ADR-0038).
    crate::http::policy::HttpPolicy: FromRef<S>,
    sqlx::PgPool: FromRef<S>,
    // Invitations par e-mail (ADR-0046).
    crate::mail::MailState: FromRef<S>,
{
    // Ouvertes sans session (mot de passe, défi ou jeton de rafraîchissement) : débit limité
    // par adresse cliente.
    let unauthenticated = Router::new()
        .route("/auth/login", post(auth::login))
        .route("/auth/password", post(auth::change_required_password))
        .route("/auth/invitation", post(auth::accept_invitation))
        .route("/auth/password/forgot", post(auth::forgot_password))
        .route("/auth/password/reset", post(auth::reset_password))
        .route("/auth/totp/verify", post(auth::verify_totp))
        .route("/auth/totp/setup", post(auth::start_totp_setup))
        .route("/auth/totp/confirm", post(auth::confirm_totp))
        .route("/auth/refresh", post(auth::refresh))
        .route_layer(from_fn(limit_unauthenticated));
    Router::new()
        .merge(unauthenticated)
        .route("/auth/logout", post(auth::logout))
        .route("/me", get(me::get))
        .route("/me/password", put(me::change_password))
        .route("/me/elevate", post(me::elevate))
        .route("/me/totp", delete(me::disable_totp))
        .route("/me/totp/setup", post(me::start_totp_setup))
        .route("/me/totp/confirm", post(me::confirm_totp))
        .route(
            "/me/totp/recovery-codes",
            post(me::regenerate_recovery_codes),
        )
        .route("/permissions", get(permissions::catalog))
        .route("/roles", get(roles::list).post(roles::create))
        .route(
            "/roles/{id}",
            get(roles::get).patch(roles::update).delete(roles::delete),
        )
        .route("/users", get(users::list).post(users::create))
        .route(
            "/users/{id}",
            get(users::get).patch(users::update).delete(users::delete),
        )
        .route("/users/{id}/roles", put(users::set_roles))
        .route("/users/{id}/password", put(users::reset_password))
        .route("/users/{id}/invite", post(users::invite))
        .route("/users/{id}/totp", delete(users::reset_totp))
        .route("/users/{id}/sessions", delete(users::revoke_sessions))
}
