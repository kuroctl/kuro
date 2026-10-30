//! `/permissions` : catalogue, pour l'éditeur de rôles du panel.

use crate::http::extract::Json;
use axum::extract::State;
use serde::Serialize;
use utoipa::ToSchema;

use super::AdminSession;
use crate::http::error::{ApiError, Problem};
use crate::iam::application::IamState;
use crate::iam::domain::{Availability, Category, Permission};

/// Clé d'une permission du catalogue, publiée comme énumération fermée : le panel traduit
/// chacune et sa compilation échoue s'il en manque une.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(transparent)]
pub struct PermissionKey(&'static str);

impl utoipa::PartialSchema for PermissionKey {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        utoipa::openapi::schema::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::Type::String)
            .enum_values(Some(Permission::ALL.iter().map(|p| p.key())))
            .into()
    }
}

impl ToSchema for PermissionKey {}

#[derive(Debug, Serialize, ToSchema)]
pub struct PermissionDto {
    pub key: PermissionKey,
    pub label: &'static str,
    /// Action à fort impact : mise en évidence dans le panel, toujours auditée.
    pub sensitive: bool,
    pub availability: Availability,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CategoryDto {
    pub key: Category,
    pub label: &'static str,
    pub permissions: Vec<PermissionDto>,
}

/// Catalogue des permissions, par catégorie. Permission : `iam.roles.read`.
#[utoipa::path(get, path = "/api/admin/v1/permissions", tag = "admin-iam", security(("bearer" = [])),
    responses((status = 200, body = [CategoryDto]), (status = 403, body = Problem)))]
pub async fn catalog(
    State(state): State<IamState>,
    session: AdminSession,
) -> Result<Json<Vec<CategoryDto>>, ApiError> {
    session.require(&state, Permission::IamRolesRead).await?;

    let mut categories: Vec<CategoryDto> = Vec::new();
    for permission in Permission::ALL {
        let dto = PermissionDto {
            key: PermissionKey(permission.key()),
            label: permission.label(),
            sensitive: permission.is_sensitive(),
            availability: permission.availability(),
        };
        match categories
            .iter_mut()
            .find(|category| category.key == permission.category())
        {
            Some(category) => category.permissions.push(dto),
            None => categories.push(CategoryDto {
                key: permission.category(),
                label: permission.category().label(),
                permissions: vec![dto],
            }),
        }
    }
    Ok(Json(categories))
}
