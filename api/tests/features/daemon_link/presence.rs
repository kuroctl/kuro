//! Un nœud reste `online` tant qu'il envoie des heartbeats sur sa connexion courante.

use std::time::Duration;

use crate::support::link::{
    HEARTBEAT_TIMEOUT_SECS, TestApi, assert_closed, heartbeat, say_hello, send,
};

#[tokio::test]
async fn silent_node_goes_offline_after_heartbeat_timeout() {
    let api = TestApi::start().await;
    let node = api.enroll_node("node-1").await;
    let mut ws = api.connect(&node).await;
    say_hello(&mut ws).await;
    assert_eq!(api.status(node.id).await, "online");

    assert_closed(&mut ws).await;
    api.wait_for_status(node.id, "offline").await;
}

#[tokio::test]
async fn heartbeats_keep_the_node_online() {
    let api = TestApi::start().await;
    let node = api.enroll_node("node-1").await;
    let mut ws = api.connect(&node).await;
    say_hello(&mut ws).await;

    // Deux fois le délai de timeout, avec un heartbeat toutes les 500 ms.
    for _ in 0..(HEARTBEAT_TIMEOUT_SECS * 4) {
        tokio::time::sleep(Duration::from_millis(500)).await;
        send(&mut ws, heartbeat()).await;
    }

    assert_eq!(api.status(node.id).await, "online");
}

#[tokio::test]
async fn new_connection_replaces_the_previous_one() {
    let api = TestApi::start().await;
    let node = api.enroll_node("node-1").await;
    let mut first = api.connect(&node).await;
    say_hello(&mut first).await;

    let mut second = api.connect(&node).await;
    say_hello(&mut second).await;

    assert_closed(&mut first).await;
    send(&mut second, heartbeat()).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(api.status(node.id).await, "online");

    drop(second);
    api.wait_for_status(node.id, "offline").await;
}
