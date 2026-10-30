use serde::{Deserialize, Serialize};

/// Signe de vie envoyé toutes les [`crate::limits::HEARTBEAT_INTERVAL`], sans acquittement.
///
/// Payload vide (`{}`), gardé sous forme de struct pour pouvoir l'enrichir sans casser le format.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Heartbeat {}
