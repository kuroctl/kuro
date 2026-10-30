//! Emplacement physique : région, coordonnées, libellés.

/// Position sur la carte (WGS 84).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coordinates {
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Debug, thiserror::Error)]
#[error("invalid coordinates: latitude must be in [-90, 90] and longitude in [-180, 180]")]
pub struct InvalidCoordinates;

impl Coordinates {
    pub fn new(latitude: f64, longitude: f64) -> Result<Self, InvalidCoordinates> {
        let valid = latitude.is_finite()
            && longitude.is_finite()
            && (-90.0..=90.0).contains(&latitude)
            && (-180.0..=180.0).contains(&longitude);
        if valid {
            Ok(Self {
                latitude,
                longitude,
            })
        } else {
            Err(InvalidCoordinates)
        }
    }
}

/// Identifiant d'une région (`eu-west`), 64 caractères au plus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionSlug(String);

#[derive(Debug, thiserror::Error)]
#[error("invalid region slug {0:?}: expected 1-64 chars of [a-z0-9-]")]
pub struct InvalidRegionSlug(pub String);

impl RegionSlug {
    pub fn parse(value: &str) -> Result<Self, InvalidRegionSlug> {
        if crate::validation::is_slug(value, 64) {
            Ok(Self(value.to_owned()))
        } else {
            Err(InvalidRegionSlug(value.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Libellé affiché (nom de région, libellé de nœud) : 1 à 100 caractères.
pub fn is_valid_label(value: &str) -> bool {
    crate::validation::is_display_text(value.trim(), 1, 100)
}

/// Localisation libre (« Paris – OVH GRA ») : 1 à 200 caractères.
pub fn is_valid_location(value: &str) -> bool {
    crate::validation::is_display_text(value.trim(), 1, 200)
}

/// Emplacement d'un nœud, entièrement facultatif. Sans coordonnées propres, le nœud est placé
/// au centre de sa région sur la carte.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Placement {
    pub region_id: Option<uuid::Uuid>,
    pub display_name: Option<String>,
    pub location: Option<String>,
    pub coordinates: Option<Coordinates>,
}
