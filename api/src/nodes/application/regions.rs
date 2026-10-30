//! Régions : emplacement physique des nœuds. Le slug est fixe (référencé par la CLI et les
//! scripts) ; le nom et le centre sur la carte sont modifiables.

use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::NodesError;
use super::error::constraint_violation;
use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::error_code::{ErrorCode, FieldError};
use crate::nodes::domain::{Coordinates, RegionSlug, is_valid_label};
use crate::nodes::infra::{RegionRecord, RegionRepository};

#[derive(Debug, Clone)]
pub struct NewRegion {
    pub slug: String,
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Debug, Clone, Default)]
pub struct RegionPatch {
    pub name: Option<String>,
    pub coordinates: Option<(f64, f64)>,
}

pub async fn list(pool: &PgPool) -> Result<Vec<RegionRecord>, NodesError> {
    let mut conn = pool.acquire().await?;
    Ok(RegionRepository::list(&mut conn).await?)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<RegionRecord, NodesError> {
    let mut conn = pool.acquire().await?;
    RegionRepository::get(&mut conn, id)
        .await?
        .ok_or(NodesError::NotFound)
}

pub async fn id_by_slug(pool: &PgPool, slug: &str) -> Result<Uuid, NodesError> {
    let mut conn = pool.acquire().await?;
    RegionRepository::id_by_slug(&mut conn, slug)
        .await?
        .ok_or(NodesError::UnknownRegion)
}

pub async fn create(
    pool: &PgPool,
    context: &AuditContext,
    input: NewRegion,
) -> Result<RegionRecord, NodesError> {
    let slug = RegionSlug::parse(&input.slug)
        .map_err(|_| NodesError::Invalid(FieldError::new("slug", ErrorCode::InvalidRegionSlug)))?;
    let name = input.name.trim();
    if !is_valid_label(name) {
        return Err(NodesError::Invalid(FieldError::new(
            "name",
            ErrorCode::InvalidRegionName,
        )));
    }
    let center = Coordinates::new(input.latitude, input.longitude).map_err(|_| {
        NodesError::Invalid(FieldError::new(
            "coordinates",
            ErrorCode::InvalidCoordinates,
        ))
    })?;

    let id = Uuid::now_v7();
    let mut tx = AuditedTx::begin(pool, context).await?;
    RegionRepository::insert(
        &mut tx,
        id,
        slug.as_str(),
        name,
        center.latitude,
        center.longitude,
    )
    .await
    .map_err(|error| constraint_violation(error, ErrorCode::RegionSlugTaken))?;
    let region = RegionRepository::get(&mut tx, id)
        .await?
        .ok_or(NodesError::NotFound)?;
    tx.commit(
        AuditEvent::success(AuditAction::RegionCreated)
            .target("region", id)
            .details(json!({ "slug": region.slug })),
    )
    .await?;
    Ok(region)
}

pub async fn update(
    pool: &PgPool,
    context: &AuditContext,
    id: Uuid,
    patch: RegionPatch,
) -> Result<RegionRecord, NodesError> {
    let name = patch.name.as_deref().map(str::trim);
    if name.is_some_and(|name| !is_valid_label(name)) {
        return Err(NodesError::Invalid(FieldError::new(
            "name",
            ErrorCode::InvalidRegionName,
        )));
    }
    let center = patch
        .coordinates
        .map(|(latitude, longitude)| Coordinates::new(latitude, longitude))
        .transpose()
        .map_err(|_| {
            NodesError::Invalid(FieldError::new(
                "coordinates",
                ErrorCode::InvalidCoordinates,
            ))
        })?;
    if name.is_none() && center.is_none() {
        return Err(NodesError::Unprocessable(ErrorCode::EmptyPatch));
    }

    let mut tx = AuditedTx::begin(pool, context).await?;
    RegionRepository::get(&mut tx, id)
        .await?
        .ok_or(NodesError::NotFound)?;
    RegionRepository::update(&mut tx, id, name, center.map(|c| (c.latitude, c.longitude))).await?;
    let region = RegionRepository::get(&mut tx, id)
        .await?
        .ok_or(NodesError::NotFound)?;
    tx.commit(
        AuditEvent::success(AuditAction::RegionUpdated)
            .target("region", id)
            .details(json!({ "name_changed": name.is_some(), "center_changed": center.is_some() })),
    )
    .await?;
    Ok(region)
}

/// Refusé tant que la région contient des nœuds (`region_not_empty`).
pub async fn delete(pool: &PgPool, context: &AuditContext, id: Uuid) -> Result<(), NodesError> {
    let mut tx = AuditedTx::begin(pool, context).await?;
    let region = RegionRepository::get(&mut tx, id)
        .await?
        .ok_or(NodesError::NotFound)?;
    RegionRepository::delete(&mut tx, id)
        .await
        .map_err(|error| constraint_violation(error, ErrorCode::RegionSlugTaken))?;
    tx.commit(
        AuditEvent::success(AuditAction::RegionDeleted)
            .target("region", id)
            .details(json!({ "slug": region.slug })),
    )
    .await?;
    Ok(())
}
