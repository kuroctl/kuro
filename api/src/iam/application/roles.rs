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
/// Rôle livré à la mise en route (ADR-0045) : une hiérarchie prête à l'emploi, modifiable ou
/// supprimable ensuite comme tout rôle.
pub struct DefaultRole {
    pub name: &'static str,
    pub description: &'static str,
    pub color: &'static str,
    pub permissions: &'static [Permission],
}

/// Trois rôles sous le super admin : l'opérateur gère la flotte, le support agit sur les
/// serveurs sans rien détruire, l'observateur lit tout.
pub const DEFAULT_ROLES: [DefaultRole; 3] = [
    DefaultRole {
        name: "operator",
        description: "Gère les nœuds, les serveurs et les prefabs ; ni les comptes, ni les paramètres.",
        color: "#f0730e",
        permissions: &[
            Permission::NodesRead,
            Permission::NodesCreate,
            Permission::NodesUpdate,
            Permission::NodesRegionsManage,
            Permission::NodesEnrollment,
            Permission::NodesRevoke,
            Permission::NodesDelete,
            Permission::NodesAllocationsRead,
            Permission::NodesAllocationsManage,
            Permission::ServersRead,
            Permission::ServersCreate,
            Permission::ServersUpdate,
            Permission::ServersDelete,
            Permission::ServersPower,
            Permission::ServersReinstall,
            Permission::ServersUpgrade,
            Permission::ServersConsoleRead,
            Permission::ServersConsoleWrite,
            Permission::ServersActionsRun,
            Permission::ServersFilesRead,
            Permission::ServersFilesWrite,
            Permission::ServersFilesSftp,
            Permission::ServersBackupsRead,
            Permission::ServersBackupsCreate,
            Permission::ServersBackupsRestore,
            Permission::ServersBackupsDelete,
            Permission::ServersBackupsDownload,
            Permission::ServersSchedulesRead,
            Permission::ServersSchedulesManage,
            Permission::ServersNotificationsRead,
            Permission::ServersNotificationsManage,
            Permission::ServersContentRead,
            Permission::ServersContentManage,
            Permission::ServersSuspend,
            Permission::PrefabsRead,
            Permission::PrefabsManage,
            Permission::AuditRead,
            Permission::SystemNotificationsRead,
        ],
    },
    DefaultRole {
        name: "support",
        description: "Agit sur les serveurs au quotidien (démarrage, console, sauvegardes, fichiers en lecture) sans rien détruire.",
        color: "#3b9ddd",
        permissions: &[
            Permission::NodesRead,
            Permission::NodesAllocationsRead,
            Permission::ServersRead,
            Permission::ServersPower,
            Permission::ServersConsoleRead,
            Permission::ServersConsoleWrite,
            Permission::ServersFilesRead,
            Permission::ServersBackupsRead,
            Permission::ServersBackupsCreate,
            Permission::ServersSchedulesRead,
            Permission::ServersNotificationsRead,
            Permission::ServersContentRead,
            Permission::PrefabsRead,
        ],
    },
    DefaultRole {
        name: "viewer",
        description: "Lit tout, ne change rien : serveurs, nœuds, prefabs, comptes, paramètres, journal d'audit.",
        color: "#8a8f9f",
        permissions: &[
            Permission::NodesRead,
            Permission::NodesAllocationsRead,
            Permission::ServersRead,
            Permission::ServersConsoleRead,
            Permission::ServersBackupsRead,
            Permission::ServersSchedulesRead,
            Permission::ServersNotificationsRead,
            Permission::ServersContentRead,
            Permission::PrefabsRead,
            Permission::IamUsersRead,
            Permission::IamRolesRead,
            Permission::AuditRead,
            Permission::SystemSettingsRead,
            Permission::SystemNotificationsRead,
        ],
    },
];

/// Crée les rôles livrés une seule fois par installation (marque `default_roles_seeded_at`) :
/// à la mise en route, ou au premier démarrage d'une installation antérieure. Un rôle renommé
/// ou supprimé ensuite n'est jamais recréé. Rend les noms créés.
pub async fn seed_defaults_once(
    state: &IamState,
    context: &AuditContext,
) -> Result<Vec<&'static str>, IamError> {
    let seeded: Option<time::OffsetDateTime> =
        sqlx::query_scalar!("SELECT default_roles_seeded_at FROM platform_settings")
            .fetch_one(&state.pool)
            .await?;
    if seeded.is_some() {
        return Ok(Vec::new());
    }
    let created = seed_defaults(state, context).await?;
    sqlx::query!("UPDATE platform_settings SET default_roles_seeded_at = now()")
        .execute(&state.pool)
        .await?;
    Ok(created)
}

/// Crée les rôles livrés qui n'existent pas encore (par nom) ; rend les noms créés.
pub async fn seed_defaults(
    state: &IamState,
    context: &AuditContext,
) -> Result<Vec<&'static str>, IamError> {
    let existing: Vec<String> = list(state)
        .await?
        .into_iter()
        .map(|role| role.name)
        .collect();
    let mut created = Vec::new();
    for role in &DEFAULT_ROLES {
        if existing.iter().any(|name| name == role.name) {
            continue;
        }
        create(
            state,
            &Authority::Host,
            context,
            NewRole {
                name: role.name.to_owned(),
                description: role.description.to_owned(),
                color: Some(role.color.to_owned()),
                permissions: role
                    .permissions
                    .iter()
                    .map(|permission| permission.key().to_owned())
                    .collect(),
            },
        )
        .await?;
        created.push(role.name);
    }
    Ok(created)
}

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
