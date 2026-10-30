//! Tests de fonctionnalités du protocole : format sur le fil et règles de décodage.
// Code de test : les helpers hors `#[test]` peuvent paniquer comme les tests eux-mêmes.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod backup;
mod codec;
mod console;
mod content;
mod direction;
mod enroll;
mod envelope;
mod error;
mod files;
mod heartbeat;
mod hello;
mod hello_ack;
mod install;
mod server_commands;
mod server_events;
mod support;
mod version;
