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
    /// L'API répond aux requêtes `sftp.*` (ADR-0047) : sans ce drapeau (API plus ancienne), le
    /// daemon n'en envoie aucune, qui fermerait le lien.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sftp: bool,
    /// Rétention de l'historique des fichiers (ADR-0049) ; absente : valeurs par défaut.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_history: Option<crate::api::node::HistoryPolicy>,
}
