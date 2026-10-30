//! Version du protocole, annoncée par le daemon dans `hello`.
//!
//! Ajouter un champ ne change pas la version (les champs inconnus sont ignorés).
//! Tout changement incompatible l'incrémente.

pub const PROTOCOL_VERSION: u16 = 1;

/// Versions acceptées par cette build.
pub fn is_supported(version: u16) -> bool {
    version == PROTOCOL_VERSION
}
