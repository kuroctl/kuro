//! Rôles dynamiques. Anti-escalade : on ne crée, ne modifie ni ne supprime un rôle que si l'on
//! détient toutes ses permissions (avant et après modification). Le rôle système est intouchable.

use serde_json::json;
use sqlx::PgConnection;
use uuid::Uuid;

use super::error::unique_violation;
use super::{IamError, IamState};
use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::error_code::{ErrorCode, FieldError};
use crate::iam::domain::{
    Authority, Availability, Permission, PermissionSet, RoleColor, RoleName, WILDCARD,
    is_valid_description,
};
use crate::iam::infra::{RoleRecord, RoleRepository};

#[derive(Debug)]
pub struct NewRole {
    pub name: String,
    pub description: String,
    /// `#rrggbb` ; ardoise neutre si absente.
    pub color: Option<String>,
    pub permissions: Vec<String>,
}

#[derive(Debug, Default)]
pub struct RolePatch {
    pub name: Option<String>,
    pub description: Option<String>,
    pub color: Option<String>,
    pub permissions: Option<Vec<String>>,
}

pub async fn list(state: &IamState) -> Result<Vec<RoleRecord>, IamError> {
    let mut conn = state.pool.acquire().await?;
    Ok(RoleRepository::list(&mut conn).await?)
}

pub async fn get(state: &IamState, id: Uuid) -> Result<RoleRecord, IamError> {
    let mut conn = state.pool.acquire().await?;
    load(&mut conn, id).await
}

pub async fn create(
    state: &IamState,
    authority: &Authority,
    context: &AuditContext,
    input: NewRole,
) -> Result<RoleRecord, IamError> {
    let name = RoleName::parse(&input.name)
        .map_err(|_| IamError::Invalid(FieldError::new("name", ErrorCode::InvalidRoleName)))?;
    if !is_valid_description(&input.description) {
        return Err(IamError::Invalid(FieldError::new(
            "description",
            ErrorCode::InvalidDescription,
        )));
    }
    let color = input
        .color
        .as_deref()
        .map(parse_color)
        .transpose()?
        .unwrap_or_default();
    let permissions = parse_assignable(&input.permissions)?;
    if !authority.covers(&permissions) {
        return Err(IamError::Forbidden(
            ErrorCode::CannotGrantPermissionsYouLack,
        ));
    }

    let id = Uuid::now_v7();
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    RoleRepository::insert(
        &mut tx,
        id,
        name.as_str(),
        &input.description,
        color.as_str(),
    )
    .await
    .map_err(|error| unique_violation(error, ErrorCode::RoleNameTaken))?;
    RoleRepository::set_permissions(&mut tx, id, &permissions.keys()).await?;
    let role = load(&mut tx, id).await?;
    tx.commit(
        AuditEvent::success(AuditAction::RoleCreated)
            .target("role", id)
            .details(json!({ "name": role.name, "permissions": permissions.keys() })),
    )
    .await?;
    Ok(role)
}

pub async fn update(
    state: &IamState,
    authority: &Authority,
    context: &AuditContext,
    id: Uuid,
    patch: RolePatch,
) -> Result<RoleRecord, IamError> {
    let name = patch
        .name
        .as_deref()
        .map(RoleName::parse)
        .transpose()
        .map_err(|_| IamError::Invalid(FieldError::new("name", ErrorCode::InvalidRoleName)))?;
    if patch
        .description
        .as_deref()
        .is_some_and(|description| !is_valid_description(description))
    {
        return Err(IamError::Invalid(FieldError::new(
            "description",
            ErrorCode::InvalidDescription,
        )));
    }
    let color = patch.color.as_deref().map(parse_color).transpose()?;
    let permissions = patch
        .permissions
        .as_deref()
        .map(parse_assignable)
        .transpose()?;
    if name.is_none() && patch.description.is_none() && color.is_none() && permissions.is_none() {
        return Err(IamError::Unprocessable(ErrorCode::EmptyPatch));
    }

    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let current = load(&mut tx, id).await?;
    let current_permissions = ensure_editable(authority, &current)?;
    if let Some(permissions) = &permissions
        && !authority.covers(permissions)
    {
        return Err(IamError::Forbidden(
            ErrorCode::CannotGrantPermissionsYouLack,
        ));
    }

    RoleRepository::update(
        &mut tx,
        id,
        name.as_ref().map(RoleName::as_str),
        patch.description.as_deref(),
        color.as_ref().map(RoleColor::as_str),
    )
    .await
    .map_err(|error| unique_violation(error, ErrorCode::RoleNameTaken))?;
    if let Some(permissions) = &permissions {
        RoleRepository::set_permissions(&mut tx, id, &permissions.keys()).await?;
    }
    let role = load(&mut tx, id).await?;

    let (added, removed) = match &permissions {
        Some(next) => (
            difference(next, &current_permissions),
            difference(&current_permissions, next),
        ),
        None => (Vec::new(), Vec::new()),
    };
    tx.commit(
        AuditEvent::success(AuditAction::RoleUpdated)
            .target("role", id)
            .details(json!({
                "renamed": name.is_some().then_some([current.name.as_str(), role.name.as_str()]),
                "description_changed": patch.description.is_some(),
                "color": color.as_ref().map(RoleColor::as_str),
                "added": added,
                "removed": removed,
            })),
    )
    .await?;
    Ok(role)
}

/// Les comptes qui portaient le rôle le perdent.
pub async fn delete(
    state: &IamState,
    authority: &Authority,
    context: &AuditContext,
    id: Uuid,
) -> Result<(), IamError> {
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let current = load(&mut tx, id).await?;
    ensure_editable(authority, &current)?;
    RoleRepository::delete(&mut tx, id).await?;
    tx.commit(
        AuditEvent::success(AuditAction::RoleDeleted)
            .target("role", id)
            .details(json!({ "name": current.name, "members": current.members })),
    )
    .await?;
    Ok(())
}

fn parse_color(value: &str) -> Result<RoleColor, IamError> {
    RoleColor::parse(value)
        .map_err(|_| IamError::Invalid(FieldError::new("color", ErrorCode::InvalidColor)))
}

/// Permissions demandées pour un rôle : connues, attribuables, jamais le joker.
fn parse_assignable(keys: &[String]) -> Result<PermissionSet, IamError> {
    let mut permissions = Vec::with_capacity(keys.len());
    for key in keys {
        if key == WILDCARD {
            return Err(IamError::PermissionNotAssignable(key.clone()));
        }
        let permission =
            Permission::from_key(key).ok_or_else(|| IamError::UnknownPermission(key.clone()))?;
        if permission.availability() != Availability::Available {
            return Err(IamError::PermissionNotAssignable(key.clone()));
        }
        permissions.push(permission);
    }
    Ok(PermissionSet::from_permissions(permissions))
}

/// Rôle modifiable par cet acteur ; retourne ses permissions actuelles.
fn ensure_editable(authority: &Authority, role: &RoleRecord) -> Result<PermissionSet, IamError> {
    if role.is_system {
        return Err(IamError::Forbidden(ErrorCode::SystemRole));
    }
    let permissions = PermissionSet::from_keys(&role.permissions);
    if !authority.covers(&permissions) {
        return Err(IamError::Forbidden(
            ErrorCode::InsufficientPermissionsForRole,
        ));
    }
    Ok(permissions)
}

fn difference(left: &PermissionSet, right: &PermissionSet) -> Vec<&'static str> {
    left.permissions()
        .filter(|permission| !right.contains(*permission))
        .map(Permission::key)
        .collect()
}

async fn load(conn: &mut PgConnection, id: Uuid) -> Result<RoleRecord, IamError> {
    RoleRepository::get(conn, id)
        .await?
        .ok_or(IamError::NotFound)
}
