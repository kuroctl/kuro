//! Tests de fonctionnalités des prefabs : prefabs officiels, accord avec le JSON Schema publié,
//! gabarits, réglages et résolution en `server.deploy`.
// Code de test : les helpers hors `#[test]` peuvent paniquer comme les tests eux-mêmes.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod archive;
mod backup;
mod compose;
mod egg;
mod official;
mod resolve;
mod resources;
mod schema;
mod settings;
mod support;
mod templates;
