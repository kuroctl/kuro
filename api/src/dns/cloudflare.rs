//! Adaptateur Cloudflare (API v4). Jeton en `Authorization: Bearer`, limité à
//! `Zone › DNS › Edit` sur la zone. Les enregistrements ne passent jamais par le proxy
//! Cloudflare (`proxied: false`) : le trafic d'un jeu ne le traverse pas.

use std::time::Duration;

use futures_util::future::BoxFuture;
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HOST, USER_AGENT};
use hyper::{Method, Request, StatusCode};
use serde_json::{Value, json};

use super::domain::{Desired, RecordKind};
use super::provider::{DnsProvider, ProviderError, Remote, ZoneAccess};
use crate::http::outbound::{Https, Target};

const HOST_NAME: &str = "api.cloudflare.com";
const TIMEOUT: Duration = Duration::from_secs(15);
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
/// Enregistrements par page de liste (maximum de l'API).
const PER_PAGE: u32 = 5000;

#[derive(Debug, Clone)]
pub struct Cloudflare {
    https: Https,
}

impl Cloudflare {
    pub fn new() -> Result<Self, rustls::Error> {
        Ok(Self {
            https: Https::new()?,
        })
    }
}

fn unavailable(error: &dyn std::fmt::Display) -> ProviderError {
    ProviderError::Unavailable(error.to_string())
}

/// Corps d'un enregistrement : un SRV porte ses champs dans `data`.
fn body(record: &Desired, ttl: u32) -> Value {
    if record.kind == RecordKind::Srv {
        let mut parts = record.content.split_whitespace();
        let mut next = || parts.next().unwrap_or_default().to_owned();
        let (priority, weight, port, target) = (next(), next(), next(), next());
        return json!({
            "type": "SRV",
            "name": record.name,
            "ttl": ttl,
            "data": {
                "priority": priority.parse::<u16>().unwrap_or(0),
                "weight": weight.parse::<u16>().unwrap_or(0),
                "port": port.parse::<u16>().unwrap_or(0),
                "target": target,
            },
        });
    }
    json!({
        "type": record.kind.as_str(),
        "name": record.name,
        "content": record.content,
        "ttl": ttl,
        "proxied": false,
    })
}

/// Enregistrement lu dans une réponse de liste ; ceux d'autres types sont ignorés.
fn remote(value: &Value) -> Option<Remote> {
    let kind = RecordKind::parse(value["type"].as_str()?)?;
    let content = if kind == RecordKind::Srv {
        let data = &value["data"];
        format!(
            "{} {} {} {}",
            data["priority"].as_u64()?,
            data["weight"].as_u64()?,
            data["port"].as_u64()?,
            data["target"].as_str()?.trim_end_matches('.'),
        )
    } else {
        value["content"].as_str()?.to_owned()
    };
    Some(Remote {
        id: value["id"].as_str()?.to_owned(),
        record: Desired {
            kind,
            name: value["name"].as_str()?.to_owned(),
            content,
        },
    })
}

impl Cloudflare {
    async fn call(
        &self,
        method: Method,
        path: &str,
        token: &str,
        payload: Option<Value>,
    ) -> Result<Value, ProviderError> {
        let request = Request::builder()
            .method(method)
            .uri(format!("/client/v4{path}"))
            .header(HOST, HOST_NAME)
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .header(USER_AGENT, concat!("kuro-api/", env!("CARGO_PKG_VERSION")))
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "application/json")
            .body(Full::new(Bytes::from(
                payload.map(|p| p.to_string()).unwrap_or_default(),
            )))
            .map_err(|e| unavailable(&e))?;
        let (status, bytes) = self
            .https
            .send(
                HOST_NAME,
                Target::Host(HOST_NAME.to_owned(), 443),
                request,
                MAX_RESPONSE_BYTES,
                TIMEOUT,
            )
            .await
            .map_err(|e| unavailable(&e))?;
        let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(ProviderError::Unauthorized),
            StatusCode::NOT_FOUND => Err(ProviderError::ZoneNotFound),
            status if status.is_success() && body["success"] == true => Ok(body),
            status if status.is_client_error() => Err(ProviderError::Rejected(
                body["errors"][0]["message"]
                    .as_str()
                    .unwrap_or("rejected")
                    .to_owned(),
            )),
            status => Err(ProviderError::Unavailable(format!("answered {status}"))),
        }
    }
}

impl DnsProvider for Cloudflare {
    fn find_zone<'a>(
        &'a self,
        token: &'a str,
        domain: &'a str,
    ) -> BoxFuture<'a, Result<String, ProviderError>> {
        Box::pin(async move {
            // `play.kuroctl.com`, puis `kuroctl.com` : la première zone trouvée la contient.
            let labels: Vec<&str> = domain.split('.').collect();
            for start in 0..labels.len().saturating_sub(1) {
                let candidate = labels[start..].join(".");
                let found = self
                    .call(
                        Method::GET,
                        &format!("/zones?name={candidate}"),
                        token,
                        None,
                    )
                    .await?;
                if let Some(id) = found["result"][0]["id"].as_str() {
                    return Ok(id.to_owned());
                }
            }
            Err(ProviderError::ZoneNotFound)
        })
    }

    fn list<'a>(
        &'a self,
        access: &'a ZoneAccess,
    ) -> BoxFuture<'a, Result<Vec<Remote>, ProviderError>> {
        Box::pin(async move {
            let mut records = Vec::new();
            let mut page: u32 = 1;
            loop {
                let body = self
                    .call(
                        Method::GET,
                        &format!(
                            "/zones/{}/dns_records?per_page={PER_PAGE}&page={page}",
                            access.zone
                        ),
                        &access.token,
                        None,
                    )
                    .await?;
                records.extend(
                    body["result"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(remote),
                );
                let pages = body["result_info"]["total_pages"].as_u64().unwrap_or(1);
                if u64::from(page) >= pages {
                    return Ok(records);
                }
                page += 1;
            }
        })
    }

    fn create<'a>(
        &'a self,
        access: &'a ZoneAccess,
        record: &'a Desired,
    ) -> BoxFuture<'a, Result<String, ProviderError>> {
        Box::pin(async move {
            let body = self
                .call(
                    Method::POST,
                    &format!("/zones/{}/dns_records", access.zone),
                    &access.token,
                    Some(body(record, access.ttl)),
                )
                .await?;
            body["result"]["id"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| ProviderError::Unavailable("no record id".to_owned()))
        })
    }

    fn update<'a>(
        &'a self,
        access: &'a ZoneAccess,
        id: &'a str,
        record: &'a Desired,
    ) -> BoxFuture<'a, Result<(), ProviderError>> {
        Box::pin(async move {
            self.call(
                Method::PUT,
                &format!("/zones/{}/dns_records/{id}", access.zone),
                &access.token,
                Some(body(record, access.ttl)),
            )
            .await
            .map(|_| ())
        })
    }

    fn delete<'a>(
        &'a self,
        access: &'a ZoneAccess,
        id: &'a str,
    ) -> BoxFuture<'a, Result<(), ProviderError>> {
        Box::pin(async move {
            match self
                .call(
                    Method::DELETE,
                    &format!("/zones/{}/dns_records/{id}", access.zone),
                    &access.token,
                    None,
                )
                .await
            {
                Ok(_) | Err(ProviderError::ZoneNotFound) => Ok(()),
                Err(error) => Err(error),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_are_never_proxied_and_srv_carries_its_fields() {
        let a = Desired {
            kind: RecordKind::A,
            name: "dev-1.nodes.play.kuroctl.com".to_owned(),
            content: "203.0.113.7".to_owned(),
        };
        assert_eq!(body(&a, 300)["proxied"], false);
        let srv = Desired {
            kind: RecordKind::Srv,
            name: "_minecraft._tcp.survie.play.kuroctl.com".to_owned(),
            content: "0 0 25568 dev-1.nodes.play.kuroctl.com".to_owned(),
        };
        let body = body(&srv, 300);
        assert_eq!(body["data"]["port"], 25568);
        assert_eq!(body["data"]["target"], "dev-1.nodes.play.kuroctl.com");
        // Relu tel qu'écrit.
        let read =
            remote(&json!({ "id": "r1", "type": "SRV", "name": srv.name, "data": body["data"] }))
                .unwrap();
        assert_eq!(read.record, srv);
    }
}
