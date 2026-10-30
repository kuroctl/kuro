//! Outillage commun : ids fixes et comparaison au JSON de référence.

use kuro_protocol::{Envelope, MessageId, decode, encode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use uuid::Uuid;

pub const ID: &str = "0192f3a4-5b6c-7d8e-9f01-23456789abcd";
pub const REQUEST_ID: &str = "0192f3a4-0000-7000-8000-000000000001";

pub fn id(text: &str) -> MessageId {
    MessageId::from(Uuid::parse_str(text).unwrap())
}

/// Vérifie l'encodage exact, puis que le décodage redonne l'enveloppe d'origine.
pub fn assert_wire_format<M>(envelope: &Envelope<M>, expected: Value)
where
    M: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let text = encode(envelope).unwrap();
    let actual: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(actual, expected);

    let decoded: Envelope<M> = decode(&text).unwrap();
    assert_eq!(&decoded, envelope);
}
