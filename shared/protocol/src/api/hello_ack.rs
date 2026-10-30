use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Acceptation du `hello` : le nœud est `online`.
///
/// Les délais sont imposés par l'API, le daemon les applique tels quels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloAck {
    /// Identifiant du nœud, déduit du certificat client.
    pub node_id: Uuid,
    pub heartbeat_interval_secs: u64,
    pub heartbeat_timeout_secs: u64,
}
