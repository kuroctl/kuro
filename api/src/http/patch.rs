//! PATCH JSON à trois états pour les champs effaçables :
//! champ absent → `None` (inchangé), `null` → `Some(None)` (effacé), valeur → `Some(Some(v))`.
//!
//! ```ignore
//! #[serde(default, deserialize_with = "crate::http::patch::nullable")]
//! pub location: Option<Option<String>>,
//! ```

use serde::{Deserialize, Deserializer};

pub fn nullable<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}
