//! Le premier message doit être un `hello` valide, reçu à temps.

use crate::support::link::{
    HEARTBEAT_INTERVAL_SECS, HEARTBEAT_TIMEOUT_SECS, TestApi, assert_closed, heartbeat, hello,
    recv, say_hello, send,
};

#[tokio::test]
async fn hello_is_acknowledged_with_node_id_and_link_timings() {
    let api = TestApi::start().await;
    let node = api.enroll_node("node-1").await;
    let mut ws = api.connect(&node).await;
    let hello_id = uuid::Uuid::now_v7().to_string();

    send(&mut ws, hello(&hello_id, kuro_protocol::PROTOCOL_VERSION)).await;
    let ack = recv(&mut ws).await.unwrap();

    assert_eq!(ack["type"], "hello.ack");
    assert_eq!(ack["reply_to"], hello_id);
    assert_eq!(ack["payload"]["node_id"], node.id.to_string());
    assert_eq!(
        ack["payload"]["heartbeat_interval_secs"],
        HEARTBEAT_INTERVAL_SECS
    );
    assert_eq!(
        ack["payload"]["heartbeat_timeout_secs"],
        HEARTBEAT_TIMEOUT_SECS
    );
    assert_eq!(api.status(node.id).await, "online");
    let sync = recv(&mut ws).await.unwrap();
    assert_eq!(sync["type"], "server.sync");
    assert_eq!(sync["payload"]["server_ids"], serde_json::json!([]));
}

#[tokio::test]
async fn unsupported_protocol_version_is_rejected() {
    let api = TestApi::start().await;
    let node = api.enroll_node("node-1").await;
    let mut ws = api.connect(&node).await;
    let hello_id = uuid::Uuid::now_v7().to_string();

    send(
        &mut ws,
        hello(&hello_id, kuro_protocol::PROTOCOL_VERSION + 1),
    )
    .await;
    let error = recv(&mut ws).await.unwrap();

    assert_eq!(error["type"], "error");
    assert_eq!(error["reply_to"], hello_id);
    assert_eq!(error["payload"]["code"], "unsupported_protocol_version");
    assert_closed(&mut ws).await;
    assert_eq!(api.status(node.id).await, "offline");
}

#[tokio::test]
async fn heartbeat_before_hello_is_rejected() {
    let api = TestApi::start().await;
    let node = api.enroll_node("node-1").await;
    let mut ws = api.connect(&node).await;

    send(&mut ws, heartbeat()).await;
    let error = recv(&mut ws).await.unwrap();

    assert_eq!(error["payload"]["code"], "unexpected_message");
    assert_closed(&mut ws).await;
    assert_eq!(api.status(node.id).await, "offline");
}

#[tokio::test]
async fn malformed_first_message_is_rejected() {
    let api = TestApi::start().await;
    let node = api.enroll_node("node-1").await;
    let mut ws = api.connect(&node).await;

    send(&mut ws, serde_json::json!({ "type": "hello" })).await;
    let error = recv(&mut ws).await.unwrap();

    assert_eq!(error["payload"]["code"], "invalid_message");
    assert_closed(&mut ws).await;
}

#[tokio::test]
async fn missing_hello_closes_the_connection() {
    let api = TestApi::start().await;
    let node = api.enroll_node("node-1").await;
    let mut ws = api.connect(&node).await;

    let error = recv(&mut ws).await.unwrap();

    assert_eq!(error["payload"]["code"], "unexpected_message");
    assert_closed(&mut ws).await;
}

#[tokio::test]
async fn second_hello_is_rejected_without_closing() {
    let api = TestApi::start().await;
    let node = api.enroll_node("node-1").await;
    let mut ws = api.connect(&node).await;
    say_hello(&mut ws).await;

    send(
        &mut ws,
        hello(
            &uuid::Uuid::now_v7().to_string(),
            kuro_protocol::PROTOCOL_VERSION,
        ),
    )
    .await;
    let error = recv(&mut ws).await.unwrap();

    assert_eq!(error["payload"]["code"], "unexpected_message");
    send(&mut ws, heartbeat()).await;
    assert_eq!(api.status(node.id).await, "online");
}
