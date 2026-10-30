//! Actions d'un prefab (ADR-0039) : une commande typée, rendue par l'API avec des paramètres
//! validés contre leur déclaration, exécutée par le daemon dans le conteneur du service en
//! marche. La sortie revient à l'appelant, tronquée ; elle n'entre jamais dans l'audit, qui
//! garde la clé de l'action, ses paramètres et le code de sortie.

use kuro_prefab::ActionError;
use kuro_protocol::ApiMessage;
use kuro_protocol::api::server::ServerAction;
use kuro_protocol::daemon::DaemonMessage;
use kuro_protocol::daemon::server::{CommandErrorCode, CommandResult};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::compat::DaemonFeature;
use super::{ServersError, ServersState, get};
use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::error_code::{ErrorCode, FieldError};
use crate::nodes::domain::NodeId;
use crate::nodes::infra::NodeRepository;
use crate::servers::domain::ServerState;

/// Issue d'une action : code de sortie et sortie du processus (stdout et stderr), tronquée par
/// le daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionOutcome {
    pub exit_code: i64,
    pub output: String,
    pub truncated: bool,
}

/// Lance l'action `key` du prefab du serveur avec `params`. Le serveur doit être en marche ;
/// le daemon refuse pendant une autre opération.
pub async fn run(
    state: &ServersState,
    context: &AuditContext,
    id: Uuid,
    key: &str,
    params: &Map<String, Value>,
) -> Result<ActionOutcome, ServersError> {
    let server = get(state, id).await?.server;
    super::manage::ensure_not_suspended(&server)?;
    if server.state != ServerState::Running.as_str()
        && server.state != ServerState::Starting.as_str()
    {
        return Err(ServersError::Conflict(ErrorCode::ServerNotRunning));
    }
    // La commande est rendue comme au déploiement (réglages, adresse du site…), avec les
    // paramètres en plus ; le daemon reçoit une liste d'arguments finale.
    let (package, inputs) = super::deploy::inputs(state, &server).await?;
    let action = kuro_prefab::resolve_action(&package, &inputs.deployment(), key, params).map_err(
        |error| match error {
            ActionError::UnknownAction(_) => ServersError::Missing(ErrorCode::UnknownAction),
            ActionError::UnknownParam(name)
            | ActionError::MissingParam(name)
            | ActionError::InvalidParam { name, .. } => ServersError::Invalid(FieldError::new(
                format!("params.{name}"),
                ErrorCode::InvalidActionParam,
            )),
            ActionError::Resolve(error) => ServersError::Internal(error.to_string()),
        },
    )?;

    let mut conn = state.pool.acquire().await?;
    let node = NodeRepository::get(&mut conn, server.node_id).await?;
    drop(conn);
    if let Some(node) = &node {
        super::compat::check_feature(
            &node.name,
            node.daemon_version.as_deref(),
            DaemonFeature::Actions,
        )?;
    }

    let reply = state
        .commands
        .request(
            NodeId::from(server.node_id),
            ApiMessage::ServerAction(ServerAction {
                server_id: id,
                action: key.to_owned(),
                service: action.service,
                command: action.command,
                timeout_secs: action.timeout_secs,
            }),
        )
        .await?;
    let outcome = match reply {
        DaemonMessage::ActionResult(result) => ActionOutcome {
            exit_code: result.exit_code,
            output: result.output,
            truncated: result.truncated,
        },
        DaemonMessage::CommandResult(CommandResult::Error {
            code: CommandErrorCode::NotRunning,
            ..
        }) => return Err(ServersError::Conflict(ErrorCode::ServerNotRunning)),
        DaemonMessage::CommandResult(result) => {
            super::deploy::accepted(result)?;
            return Err(ServersError::Daemon {
                code: ErrorCode::DaemonError,
                message: "the node accepted the action without a result".to_owned(),
            });
        }
        _ => {
            return Err(ServersError::Daemon {
                code: ErrorCode::DaemonError,
                message: "unexpected answer to the action".to_owned(),
            });
        }
    };

    // Audit une fois l'action faite : la clé, les paramètres (déclarés journalisables) et le
    // code de sortie ; jamais la sortie, qui peut contenir des secrets de l'application.
    let tx = AuditedTx::begin(&state.pool, context).await?;
    tx.commit(
        AuditEvent::success(AuditAction::ServerActionRan)
            .target("server", id)
            .details(json!({
                "action": key,
                "params": Value::Object(params.clone()),
                "exit_code": outcome.exit_code,
            })),
    )
    .await?;
    Ok(outcome)
}

/// Une action planifiable : déclarée `schedulable`, sans paramètre obligatoire sans défaut.
pub fn is_schedulable(manifest: &kuro_prefab::Manifest, key: &str) -> bool {
    manifest.actions.get(key).is_some_and(|action| {
        action.schedulable
            && !action
                .params
                .values()
                .any(|field| field.required && field.default.is_none())
    })
}
