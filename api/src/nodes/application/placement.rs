//! Emplacement et libellé d'un nœud : validation commune à la création (enrôlement) et à la
//! modification.

use serde_json::json;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::NodesError;
use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::error_code::{ErrorCode, FieldError};
use crate::nodes::domain::{Coordinates, Placement, is_valid_label, is_valid_location};
use crate::nodes::infra::{NodeRecord, NodeRepository, PlacementPatch, RegionRepository};

/// Emplacement saisi, avant validation.
#[derive(Debug, Clone, Default)]
pub struct PlacementInput {
    pub region_id: Option<Uuid>,
    pub display_name: Option<String>,
    pub location: Option<String>,
    pub coordinates: Option<(f64, f64)>,
}

/// Modification partielle : `None` laisse inchangé, `Some(None)` efface.
#[derive(Debug, Clone, Default)]
pub struct NodePatch {
    pub region_id: Option<Option<Uuid>>,
    pub display_name: Option<Option<String>>,
    pub location: Option<Option<String>>,
    pub coordinates: Option<Option<(f64, f64)>>,
    /// Nœud de test : accepte les essais du Studio.
    pub trials: Option<bool>,
}

/// Valide un emplacement complet (création d'un nœud).
pub async fn validate(
    conn: &mut PgConnection,
    input: PlacementInput,
) -> Result<Placement, NodesError> {
    if let Some(region_id) = input.region_id {
        ensure_region(conn, region_id).await?;
    }
    Ok(Placement {
        region_id: input.region_id,
        display_name: input.display_name.map(label).transpose()?,
        location: input.location.map(location).transpose()?,
        coordinates: input.coordinates.map(coordinates).transpose()?,
    })
}

pub async fn update(
    pool: &PgPool,
    context: &AuditContext,
    id: Uuid,
    patch: NodePatch,
) -> Result<NodeRecord, NodesError> {
    let validated = PlacementPatch {
        region_id: patch.region_id,
        display_name: patch
            .display_name
            .map(|value| value.map(label).transpose())
            .transpose()?,
        location: patch
            .location
            .map(|value| value.map(location).transpose())
            .transpose()?,
        coordinates: patch
            .coordinates
            .map(|value| {
                value
                    .map(|pair| coordinates(pair).map(|c| (c.latitude, c.longitude)))
                    .transpose()
            })
            .transpose()?,
    };
    let fields: Vec<&str> = [
        ("region", validated.region_id.is_some()),
        ("display_name", validated.display_name.is_some()),
        ("location", validated.location.is_some()),
        ("coordinates", validated.coordinates.is_some()),
        ("trials", patch.trials.is_some()),
    ]
    .into_iter()
    .filter_map(|(field, changed)| changed.then_some(field))
    .collect();
    if fields.is_empty() {
        return Err(NodesError::Unprocessable(ErrorCode::EmptyPatch));
    }

    let mut tx = AuditedTx::begin(pool, context).await?;
    NodeRepository::get(&mut tx, id)
        .await?
        .ok_or(NodesError::NotFound)?;
    if let Some(Some(region_id)) = validated.region_id {
        ensure_region(&mut tx, region_id).await?;
    }
    NodeRepository::update_placement(&mut tx, id, &validated).await?;
    if let Some(trials) = patch.trials {
        NodeRepository::set_trials(&mut tx, id, trials).await?;
    }
    let node = NodeRepository::get(&mut tx, id)
        .await?
        .ok_or(NodesError::NotFound)?;
    tx.commit(
        AuditEvent::success(AuditAction::NodeUpdated)
            .target("node", id)
            .details(json!({ "fields": fields })),
    )
    .await?;
    Ok(node)
}

async fn ensure_region(conn: &mut PgConnection, region_id: Uuid) -> Result<(), NodesError> {
    if RegionRepository::exists(conn, region_id).await? {
        Ok(())
    } else {
        Err(NodesError::UnknownRegion)
    }
}

fn label(value: String) -> Result<String, NodesError> {
    let value = value.trim().to_owned();
    if is_valid_label(&value) {
        Ok(value)
    } else {
        Err(NodesError::Invalid(FieldError::new(
            "display_name",
            ErrorCode::InvalidDisplayName,
        )))
    }
}

fn location(value: String) -> Result<String, NodesError> {
    let value = value.trim().to_owned();
    if is_valid_location(&value) {
        Ok(value)
    } else {
        Err(NodesError::Invalid(FieldError::new(
            "location",
            ErrorCode::InvalidLocation,
        )))
    }
}

fn coordinates((latitude, longitude): (f64, f64)) -> Result<Coordinates, NodesError> {
    Coordinates::new(latitude, longitude).map_err(|_| {
        NodesError::Invalid(FieldError::new(
            "coordinates",
            ErrorCode::InvalidCoordinates,
        ))
    })
}
