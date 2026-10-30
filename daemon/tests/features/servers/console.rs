//! Console et stats d'un serveur regardé (ADR-0014), sur le Docker de la machine.

use std::time::Duration;

use kuro_daemon::server::CommandError;
use kuro_protocol::DaemonMessage;
use kuro_protocol::api::server::{PowerAction, ReadySpec, ServerDeploy};
use kuro_protocol::daemon::server::ServerState;
use kuro_protocol::daemon::{ConsoleOutput, ServerStats};
use tokio::sync::mpsc;
use uuid::Uuid;

use super::support::{Node, deploy_and_start, node, service, single, wait_for};

/// Un service qui répète chaque ligne reçue sur son entrée.
fn echo(server_id: Uuid) -> ServerDeploy {
    let mut spec = single(server_id);
    spec.files.clear();
    spec.services = vec![service(
        "app",
        "echo booted; echo ready; while read line; do echo \"got:$line\"; done; exec sleep 3600",
    )];
    spec.lifecycle.ready = Some(ReadySpec::Console("^ready$".to_owned()));
    spec
}

/// Lit la console jusqu'à une ligne donnée ; retourne les lignes vues.
async fn until_line(streams: &mut mpsc::Receiver<DaemonMessage>, text: &str) -> Vec<String> {
    let mut seen = Vec::new();
    let outcome = tokio::time::timeout(Duration::from_secs(30), async {
        while let Some(message) = streams.recv().await {
            if let DaemonMessage::ConsoleLine(ConsoleOutput { lines, .. }) = message {
                for line in lines {
                    let found = line.text == text;
                    assert!(line.at_ms > 0, "line without timestamp: {}", line.text);
                    seen.push(line.text);
                    if found {
                        return;
                    }
                }
            }
        }
        panic!("stream channel closed");
    })
    .await;
    assert!(outcome.is_ok(), "line {text:?} not received, saw {seen:?}");
    seen
}

async fn next_stats(streams: &mut mpsc::Receiver<DaemonMessage>) -> ServerStats {
    let outcome = tokio::time::timeout(Duration::from_secs(30), async {
        while let Some(message) = streams.recv().await {
            if let DaemonMessage::ServerStats(stats) = message {
                return stats;
            }
        }
        panic!("stream channel closed");
    })
    .await;
    outcome.unwrap_or_else(|_| panic!("no stats received"))
}

async fn cleanup(node: &mut Node, id: Uuid) {
    node.manager.power(id, PowerAction::Kill).await.unwrap();
    wait_for(&mut node.events, id, ServerState::Offline).await;
    node.manager.delete(id).unwrap();
    wait_for(&mut node.events, id, ServerState::Deleted).await;
}

#[tokio::test]
async fn a_watched_server_streams_its_history_then_live_lines() {
    let mut node = node();
    let id = Uuid::now_v7();
    deploy_and_start(&mut node, echo(id)).await;

    node.manager.watch(id, true).unwrap();
    // Historique : ce qui précède l'abonnement.
    let history = until_line(&mut node.streams, "ready").await;
    assert!(history.contains(&"booted".to_owned()), "{history:?}");

    // Commande : écrite sur l'entrée du service principal, sa sortie arrive en direct.
    node.manager.command(id, "say hello").await.unwrap();
    until_line(&mut node.streams, "got:say hello").await;

    cleanup(&mut node, id).await;
}

#[tokio::test]
async fn a_watched_server_reports_its_consumption() {
    let mut node = node();
    let id = Uuid::now_v7();
    deploy_and_start(&mut node, echo(id)).await;

    node.manager.watch(id, true).unwrap();
    let stats = next_stats(&mut node.streams).await;

    assert_eq!(stats.server_id, id);
    assert!(stats.memory_bytes > 0);
    assert_eq!(stats.memory_limit_bytes, 64 * 1024 * 1024);

    cleanup(&mut node, id).await;
}

#[tokio::test]
async fn nothing_is_streamed_once_unwatched() {
    let mut node = node();
    let id = Uuid::now_v7();
    deploy_and_start(&mut node, echo(id)).await;

    node.manager.watch(id, true).unwrap();
    until_line(&mut node.streams, "ready").await;
    node.manager.watch(id, false).unwrap();
    // Ce qui était déjà en file peut encore arriver : on vide, puis plus rien.
    tokio::time::sleep(Duration::from_millis(300)).await;
    while node.streams.try_recv().is_ok() {}

    node.manager.command(id, "after").await.unwrap();
    let quiet = tokio::time::timeout(Duration::from_secs(5), node.streams.recv()).await;
    assert!(quiet.is_err(), "unexpected message: {quiet:?}");

    cleanup(&mut node, id).await;
}

#[tokio::test]
async fn a_command_needs_a_running_server() {
    let mut node = node();
    let id = Uuid::now_v7();
    node.manager.deploy(echo(id)).unwrap();
    wait_for(&mut node.events, id, ServerState::Offline).await;

    let refused = node.manager.command(id, "say hello").await;

    assert_eq!(refused, Err(CommandError::NotRunning));
    node.manager.delete(id).unwrap();
    wait_for(&mut node.events, id, ServerState::Deleted).await;
}

#[tokio::test]
async fn a_multiline_command_is_refused() {
    let node = node();
    let id = Uuid::now_v7();

    // Refusée avant même de chercher le serveur : aucune injection de seconde commande.
    let refused = node.manager.command(id, "say hi\nop attacker").await;

    assert!(matches!(refused, Err(CommandError::InvalidSpec(_))));
}

#[tokio::test]
async fn an_unknown_server_cannot_be_watched_or_commanded() {
    let node = node();
    let id = Uuid::now_v7();

    assert_eq!(
        node.manager.watch(id, true),
        Err(CommandError::UnknownServer)
    );
    assert_eq!(
        node.manager.command(id, "say hello").await,
        Err(CommandError::UnknownServer)
    );
}
