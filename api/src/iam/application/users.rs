//! Comptes admin : création, modification, suppression, rôles, identifiants.
//!
//! Règles d'autorisation (en plus de la permission de la route) :
//! - on ne gère que les comptes dont on détient toutes les permissions ;
//! - on ne désactive, ne supprime, ni ne change les rôles ou identifiants de son propre compte
//!   par ces voies (le profil propre a ses propres cas d'usage) ;
//! - le dernier super-admin actif reste en place.

use std::collections::BTreeSet;

use serde_json::json;
use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use super::error::unique_violation;
use super::login::{check_password, hash_password};
use super::{IamError, IamState, LOCK_DURATION, MAX_FAILED_ATTEMPTS};
use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::auth::password;
use crate::error_code::{ErrorCode, FieldError};
use crate::iam::domain::{
    Authority, Email, Permission, PermissionSet, Principal, SUPER_ADMIN_ROLE_ID,
    is_valid_display_name,
};
use crate::iam::infra::{
    AdminUserRecord, InvitationRepository, RoleRef, RoleRepository, SessionRepository,
    UserRepository,
};

#[derive(Debug, Clone)]
pub struct AdminUser {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub totp_enabled: bool,
    pub disabled: bool,
    /// Mot de passe choisi par un autre admin, à remplacer à la prochaine connexion.
    pub password_change_required: bool,
    /// Une invitation par e-mail attend d'être acceptée (ADR-0046).
    pub invitation_pending: bool,
    pub last_login_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub roles: Vec<(Uuid, String)>,
}

/// Compte connecté et ses permissions effectives.
#[derive(Debug, Clone)]
pub struct Profile {
    pub user: AdminUser,
    pub permissions: Vec<Permission>,
    pub all_permissions: bool,
}

#[derive(Debug)]
pub struct NewAdminUser {
    pub email: String,
    pub display_name: String,
    pub password: String,
}

#[derive(Debug, Default)]
pub struct UserPatch {
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub disabled: Option<bool>,
}

pub async fn list(state: &IamState) -> Result<Vec<AdminUser>, IamError> {
    let mut conn = state.pool.acquire().await?;
    let records = UserRepository::list(&mut conn).await?;
    with_roles(&mut conn, records).await
}

pub async fn get(state: &IamState, id: Uuid) -> Result<AdminUser, IamError> {
    let mut conn = state.pool.acquire().await?;
    load(&mut conn, id).await
}

/// Recherche par e-mail (outillage d'administration en CLI).
pub async fn find_by_email(state: &IamState, email: &str) -> Result<AdminUser, IamError> {
    let email = Email::parse(email)
        .map_err(|_| IamError::Invalid(FieldError::new("email", ErrorCode::InvalidEmail)))?;
    let mut conn = state.pool.acquire().await?;
    let id = UserRepository::id_by_email(&mut conn, email.as_str())
        .await?
        .ok_or(IamError::NotFound)?;
    load(&mut conn, id).await
}

pub async fn profile(state: &IamState, principal: &Principal) -> Result<Profile, IamError> {
    let user = get(state, principal.user_id).await?;
    Ok(Profile {
        user,
        permissions: principal.permissions.effective(),
        all_permissions: principal.permissions.is_all(),
    })
}

/// Un compte est créé sans rôle : les droits s'attribuent ensuite (`set_roles`, anti-escalade).
/// Créé par un admin, il devra remplacer son mot de passe à la première connexion.
pub async fn create(
    state: &IamState,
    authority: &Authority,
    context: &AuditContext,
    input: NewAdminUser,
) -> Result<AdminUser, IamError> {
    let email = Email::parse(&input.email)
        .map_err(|_| IamError::Invalid(FieldError::new("email", ErrorCode::InvalidEmail)))?;
    let display_name = input.display_name.trim();
    if !is_valid_display_name(display_name) {
        return Err(IamError::Invalid(FieldError::new(
            "display_name",
            ErrorCode::InvalidDisplayName,
        )));
    }
    let password_hash = hash_password(&input.password).await?;

    let id = Uuid::now_v7();
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    UserRepository::insert(
        &mut tx,
        id,
        email.as_str(),
        display_name,
        &password_hash,
        chosen_by_admin(authority),
    )
    .await
    .map_err(|error| unique_violation(error, ErrorCode::EmailTaken))?;
    let user = load(&mut tx, id).await?;
    tx.commit(AuditEvent::success(AuditAction::UserCreated).target("admin_user", id))
        .await?;
    Ok(user)
}

pub async fn update(
    state: &IamState,
    authority: &Authority,
    context: &AuditContext,
    id: Uuid,
    patch: UserPatch,
) -> Result<AdminUser, IamError> {
    let email = patch
        .email
        .as_deref()
        .map(Email::parse)
        .transpose()
        .map_err(|_| IamError::Invalid(FieldError::new("email", ErrorCode::InvalidEmail)))?;
    let display_name = patch.display_name.as_deref().map(str::trim);
    if display_name.is_some_and(|name| !is_valid_display_name(name)) {
        return Err(IamError::Invalid(FieldError::new(
            "display_name",
            ErrorCode::InvalidDisplayName,
        )));
    }
    if patch.disabled.is_some() && authority.is_user(id) {
        return Err(IamError::Forbidden(ErrorCode::CannotDisableSelf));
    }

    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let current = load(&mut tx, id).await?;
    ensure_manageable(&mut tx, authority, id).await?;

    let mut events = Vec::new();
    if email.is_some() || display_name.is_some() {
        UserRepository::update_profile(
            &mut tx,
            id,
            email.as_ref().map(Email::as_str),
            display_name,
        )
        .await
        .map_err(|error| unique_violation(error, ErrorCode::EmailTaken))?;
        let fields: Vec<&str> = [
            ("email", email.is_some()),
            ("display_name", display_name.is_some()),
        ]
        .into_iter()
        .filter_map(|(field, changed)| changed.then_some(field))
        .collect();
        events.push(
            AuditEvent::success(AuditAction::UserUpdated).details(json!({ "fields": fields })),
        );
    }
    match patch.disabled {
        Some(true) if !current.disabled => {
            ensure_not_last_super_admin(&mut tx, &current).await?;
            UserRepository::set_disabled(&mut tx, id, true).await?;
            SessionRepository::revoke_user(&mut tx, id, None).await?;
            events.push(AuditEvent::success(AuditAction::UserDisabled));
        }
        Some(false) if current.disabled => {
            UserRepository::set_disabled(&mut tx, id, false).await?;
            events.push(AuditEvent::success(AuditAction::UserEnabled));
        }
        _ => {}
    }

    let user = load(&mut tx, id).await?;
    commit_all(tx, events, id).await?;
    Ok(user)
}

pub async fn delete(
    state: &IamState,
    authority: &Authority,
    context: &AuditContext,
    id: Uuid,
) -> Result<(), IamError> {
    if authority.is_user(id) {
        return Err(IamError::Forbidden(ErrorCode::CannotDeleteSelf));
    }
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let current = load(&mut tx, id).await?;
    ensure_manageable(&mut tx, authority, id).await?;
    ensure_not_last_super_admin(&mut tx, &current).await?;
    UserRepository::delete(&mut tx, id).await?;
    tx.commit(AuditEvent::success(AuditAction::UserDeleted).target("admin_user", id))
        .await?;
    Ok(())
}

/// Remplace les rôles du compte.
pub async fn set_roles(
    state: &IamState,
    authority: &Authority,
    context: &AuditContext,
    id: Uuid,
    role_ids: &[Uuid],
) -> Result<AdminUser, IamError> {
    if authority.is_user(id) {
        return Err(IamError::Forbidden(ErrorCode::CannotChangeOwnRoles));
    }
    let wanted: BTreeSet<Uuid> = role_ids.iter().copied().collect();
    let wanted: Vec<Uuid> = wanted.into_iter().collect();

    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let current = load(&mut tx, id).await?;
    ensure_manageable(&mut tx, authority, id).await?;
    if usize::try_from(RoleRepository::count_existing(&mut tx, &wanted).await?) != Ok(wanted.len())
    {
        return Err(IamError::Invalid(FieldError::new(
            "role_ids",
            ErrorCode::UnknownRole,
        )));
    }
    let granted =
        PermissionSet::from_keys(RoleRepository::permission_keys(&mut tx, &wanted).await?);
    if !authority.covers(&granted) {
        return Err(IamError::Forbidden(
            ErrorCode::CannotGrantPermissionsYouLack,
        ));
    }

    let previous: BTreeSet<Uuid> = current.roles.iter().map(|(role_id, _)| *role_id).collect();
    let next: BTreeSet<Uuid> = wanted.iter().copied().collect();
    if previous.contains(&SUPER_ADMIN_ROLE_ID) && !next.contains(&SUPER_ADMIN_ROLE_ID) {
        ensure_not_last_super_admin(&mut tx, &current).await?;
    }

    UserRepository::set_roles(&mut tx, id, &wanted).await?;
    let user = load(&mut tx, id).await?;
    let added: Vec<Uuid> = next.difference(&previous).copied().collect();
    let removed: Vec<Uuid> = previous.difference(&next).copied().collect();
    tx.commit(
        AuditEvent::success(AuditAction::UserRolesChanged)
            .target("admin_user", id)
            .details(json!({ "added": added, "removed": removed })),
    )
    .await?;
    Ok(user)
}

/// Mot de passe imposé par un autre admin : sessions révoquées, verrouillage levé, à remplacer à
/// la prochaine connexion. Depuis l'hôte (secours), l'opérateur choisit le sien : rien à remplacer.
pub async fn reset_password(
    state: &IamState,
    authority: &Authority,
    context: &AuditContext,
    id: Uuid,
    new_password: &str,
) -> Result<(), IamError> {
    if authority.is_user(id) {
        return Err(IamError::Forbidden(ErrorCode::UseOwnPasswordChange));
    }
    let password_hash = hash_password(new_password).await?;
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    load(&mut tx, id).await?;
    ensure_manageable(&mut tx, authority, id).await?;
    UserRepository::set_password(&mut tx, id, &password_hash, chosen_by_admin(authority)).await?;
    SessionRepository::revoke_user(&mut tx, id, None).await?;
    // Un mot de passe imposé remplace l'invitation en cours (ADR-0046).
    InvitationRepository::revoke_open(&mut tx, id).await?;
    tx.commit(AuditEvent::success(AuditAction::UserPasswordReset).target("admin_user", id))
        .await?;
    Ok(())
}

/// Perte du téléphone : le TOTP sera reconfiguré à la prochaine connexion.
pub async fn reset_totp(
    state: &IamState,
    authority: &Authority,
    context: &AuditContext,
    id: Uuid,
) -> Result<(), IamError> {
    if authority.is_user(id) {
        return Err(IamError::Forbidden(ErrorCode::CannotResetOwnTotp));
    }
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    load(&mut tx, id).await?;
    ensure_manageable(&mut tx, authority, id).await?;
    UserRepository::reset_totp(&mut tx, id).await?;
    SessionRepository::revoke_user(&mut tx, id, None).await?;
    tx.commit(AuditEvent::success(AuditAction::UserTotpReset).target("admin_user", id))
        .await?;
    Ok(())
}

pub async fn revoke_sessions(
    state: &IamState,
    authority: &Authority,
    context: &AuditContext,
    id: Uuid,
) -> Result<u64, IamError> {
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    load(&mut tx, id).await?;
    ensure_manageable(&mut tx, authority, id).await?;
    let revoked = SessionRepository::revoke_user(&mut tx, id, None).await?;
    tx.commit(
        AuditEvent::success(AuditAction::UserSessionsRevoked)
            .target("admin_user", id)
            .details(json!({ "revoked": revoked })),
    )
    .await?;
    Ok(revoked)
}

/// Changement de son propre mot de passe : l'actuel est exigé ; les autres sessions sont fermées.
pub async fn change_own_password(
    state: &IamState,
    principal: &Principal,
    context: &AuditContext,
    current_password: &str,
    new_password: &str,
) -> Result<(), IamError> {
    password::check_policy(new_password)
        .map_err(|_| IamError::Invalid(FieldError::new("new_password", ErrorCode::WeakPassword)))?;
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let credentials = UserRepository::credentials_by_id(&mut tx, principal.user_id)
        .await?
        .ok_or(IamError::InvalidSession)?;
    if !check_password(current_password, &credentials.password_hash).await {
        let locked = UserRepository::record_failure(
            &mut tx,
            principal.user_id,
            MAX_FAILED_ATTEMPTS,
            LOCK_DURATION.as_secs_f64(),
        )
        .await?;
        let action = if locked {
            AuditAction::AccountLocked
        } else {
            AuditAction::LoginFailed
        };
        tx.commit(
            AuditEvent::failure(action)
                .target("admin_user", principal.user_id)
                .details(json!({ "reason": "wrong_current_password" })),
        )
        .await?;
        return Err(IamError::InvalidCredentials);
    }

    let password_hash = hash_password(new_password).await?;
    UserRepository::set_password(&mut tx, principal.user_id, &password_hash, false).await?;
    SessionRepository::revoke_user(&mut tx, principal.user_id, Some(principal.session_id)).await?;
    tx.commit(
        AuditEvent::success(AuditAction::OwnPasswordChanged)
            .target("admin_user", principal.user_id),
    )
    .await?;
    Ok(())
}

/// Mot de passe choisi pour quelqu'un d'autre par un admin, plutôt que par l'opérateur de l'hôte.
fn chosen_by_admin(authority: &Authority) -> bool {
    matches!(authority, Authority::Admin(_))
}

/// Anti-escalade : l'acteur détient toutes les permissions du compte visé.
pub(super) async fn ensure_manageable(
    conn: &mut PgConnection,
    authority: &Authority,
    target: Uuid,
) -> Result<(), IamError> {
    let target_permissions =
        PermissionSet::from_keys(UserRepository::permission_keys(conn, target).await?);
    if authority.covers(&target_permissions) {
        Ok(())
    } else {
        Err(IamError::Forbidden(
            ErrorCode::InsufficientPermissionsForTarget,
        ))
    }
}

/// Un super-admin actif ne peut être retiré que s'il en reste un autre.
async fn ensure_not_last_super_admin(
    conn: &mut PgConnection,
    user: &AdminUser,
) -> Result<(), IamError> {
    let is_super_admin = user
        .roles
        .iter()
        .any(|(role_id, _)| *role_id == SUPER_ADMIN_ROLE_ID);
    if is_super_admin
        && !user.disabled
        && UserRepository::other_active_super_admins(conn, user.id).await? == 0
    {
        return Err(IamError::Conflict(ErrorCode::LastSuperAdmin));
    }
    Ok(())
}

/// Valide la transaction avec ses événements (au moins un : un patch vide est refusé).
async fn commit_all(mut tx: AuditedTx, events: Vec<AuditEvent>, id: Uuid) -> Result<(), IamError> {
    let mut events = events
        .into_iter()
        .map(|event| event.target("admin_user", id));
    let Some(last) = events.next_back() else {
        return Err(IamError::Unprocessable(ErrorCode::EmptyPatch));
    };
    for event in events {
        tx.record(event).await?;
    }
    tx.commit(last).await?;
    Ok(())
}

async fn load(conn: &mut PgConnection, id: Uuid) -> Result<AdminUser, IamError> {
    let record = UserRepository::get(conn, id)
        .await?
        .ok_or(IamError::NotFound)?;
    with_roles(conn, vec![record])
        .await?
        .pop()
        .ok_or(IamError::NotFound)
}

async fn with_roles(
    conn: &mut PgConnection,
    records: Vec<AdminUserRecord>,
) -> Result<Vec<AdminUser>, IamError> {
    let ids: Vec<Uuid> = records.iter().map(|record| record.id).collect();
    let roles = RoleRepository::of_users(conn, &ids).await?;
    let pending = InvitationRepository::pending_user_ids(conn).await?;
    Ok(records
        .into_iter()
        .map(|record| AdminUser {
            roles: roles_of(&roles, record.id),
            invitation_pending: pending.contains(&record.id),
            id: record.id,
            email: record.email,
            display_name: record.display_name,
            totp_enabled: record.totp_enabled,
            disabled: record.disabled,
            password_change_required: record.password_change_required,
            last_login_at: record.last_login_at,
            created_at: record.created_at,
        })
        .collect())
}

fn roles_of(roles: &[RoleRef], user_id: Uuid) -> Vec<(Uuid, String)> {
    roles
        .iter()
        .filter(|role| role.user_id == user_id)
        .map(|role| (role.id, role.name.clone()))
        .collect()
}
