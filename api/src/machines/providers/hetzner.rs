//! Hetzner Cloud (`api.hetzner.cloud/v1`) : traduction du trait `Provider`, rien de plus. Appels
//! JSON par le client HTTPS sortant de Kuro, jeton de projet en `Bearer`.

use std::time::Duration;

use crate::http::outbound::{Https, Target};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, HOST, USER_AGENT};
use axum::http::{Method, Request, StatusCode};
use futures_util::future::BoxFuture;
use http_body_util::Full;
use serde_json::{Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use std::collections::BTreeMap;

use crate::machines::provider::{
    Listed, Location, Machine, MachineSpec, Offer, Provider, ProviderError, SnapshotState,
};

const HOST_NAME: &str = "api.hetzner.cloud";
const TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RESPONSE: usize = 2 * 1024 * 1024;
/// Image de base : une de celles que l'installeur du nœud prépare (`--full`).
const IMAGE: &str = "debian-12";

#[derive(Debug, Clone)]
pub struct Hetzner {
    token: String,
    https: Https,
}

impl Hetzner {
    pub fn new(token: String) -> Result<Self, String> {
        Ok(Self {
            token,
            https: Https::new().map_err(|e| format!("hetzner: {e}"))?,
        })
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<(StatusCode, Value), ProviderError> {
        let request = Request::builder()
            .method(method)
            .uri(format!("/v1{path}"))
            .header(HOST, HOST_NAME)
            .header(AUTHORIZATION, format!("Bearer {}", self.token))
            .header(CONTENT_TYPE, "application/json")
            .header(USER_AGENT, concat!("kuro/", env!("CARGO_PKG_VERSION")))
            .body(Full::new(
                body.map(|b| b.to_string()).unwrap_or_default().into(),
            ))
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let (status, bytes) = self
            .https
            .send(
                HOST_NAME,
                Target::Host(HOST_NAME.to_owned(), 443),
                request,
                MAX_RESPONSE,
                TIMEOUT,
            )
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let value: Value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).map_err(|e| ProviderError::Transport(e.to_string()))?
        };
        if status.is_success() || status == StatusCode::NOT_FOUND {
            return Ok((status, value));
        }
        let text = |key: &str| {
            value["error"][key]
                .as_str()
                .map_or_else(String::new, str::to_owned)
        };
        Err(ProviderError::Refused {
            code: text("code"),
            message: text("message"),
        })
    }
}

impl Provider for Hetzner {
    fn locations(&self) -> BoxFuture<'_, Result<Vec<Location>, ProviderError>> {
        Box::pin(async move {
            let (_, body) = self.call(Method::GET, "/locations", None).await?;
            Ok(locations(&body))
        })
    }

    fn offers<'a>(&'a self, location: &'a str) -> BoxFuture<'a, Result<Vec<Offer>, ProviderError>> {
        Box::pin(async move {
            let (_, types) = self
                .call(Method::GET, "/server_types?per_page=50", None)
                .await?;
            Ok(offers(&types, location))
        })
    }

    fn create(&self, spec: MachineSpec) -> BoxFuture<'_, Result<Machine, ProviderError>> {
        Box::pin(async move {
            let body = json!({
                "name": spec.name,
                "server_type": spec.offer,
                "location": spec.location,
                "image": spec.snapshot.as_deref().unwrap_or(IMAGE),
                "user_data": spec.user_data,
                "labels": spec.labels,
                "start_after_create": true,
                "public_net": { "enable_ipv4": true, "enable_ipv6": true },
            });
            let (status, body) = self.call(Method::POST, "/servers", Some(body)).await?;
            let server = &body["server"];
            match server["id"].as_u64() {
                Some(id) if status.is_success() => Ok(Machine {
                    reference: id.to_string(),
                    ipv4: server["public_net"]["ipv4"]["ip"]
                        .as_str()
                        .map(str::to_owned),
                }),
                _ => Err(ProviderError::Transport(format!(
                    "unexpected answer {status}"
                ))),
            }
        })
    }

    fn list<'a>(
        &'a self,
        label: &'a str,
        scope: (&'a str, &'a str),
    ) -> BoxFuture<'a, Result<Vec<Listed>, ProviderError>> {
        Box::pin(async move {
            let mut found = Vec::new();
            let mut page = 1;
            let (key, value) = scope;
            loop {
                let path = format!(
                    "/servers?label_selector={label},{key}%3D{value}&per_page=50&page={page}"
                );
                let (_, body) = self.call(Method::GET, &path, None).await?;
                found.extend(listed(&body, label));
                match body["meta"]["pagination"]["next_page"].as_u64() {
                    Some(next) if next > page => page = next,
                    _ => return Ok(found),
                }
            }
        })
    }

    fn delete<'a>(&'a self, reference: &'a str) -> BoxFuture<'a, Result<(), ProviderError>> {
        Box::pin(async move {
            // 404 : déjà supprimée, la suppression se rejoue sans erreur.
            self.call(Method::DELETE, &format!("/servers/{reference}"), None)
                .await
                .map(|_| ())
        })
    }

    fn snapshot<'a>(
        &'a self,
        reference: &'a str,
        labels: &'a BTreeMap<String, String>,
    ) -> BoxFuture<'a, Result<String, ProviderError>> {
        Box::pin(async move {
            let body = json!({ "type": "snapshot", "labels": labels });
            let path = format!("/servers/{reference}/actions/create_image");
            let (status, body) = self.call(Method::POST, &path, Some(body)).await?;
            match body["image"]["id"].as_u64() {
                Some(id) if status.is_success() => Ok(id.to_string()),
                _ => Err(ProviderError::Refused {
                    code: status.as_u16().to_string(),
                    message: "no snapshot created".to_owned(),
                }),
            }
        })
    }

    fn snapshot_state<'a>(
        &'a self,
        snapshot: &'a str,
    ) -> BoxFuture<'a, Result<SnapshotState, ProviderError>> {
        Box::pin(async move {
            let (status, body) = self
                .call(Method::GET, &format!("/images/{snapshot}"), None)
                .await?;
            Ok(if status == StatusCode::NOT_FOUND {
                SnapshotState::Missing
            } else if body["image"]["status"] == "available" {
                SnapshotState::Ready
            } else {
                SnapshotState::Pending
            })
        })
    }

    fn delete_snapshot<'a>(
        &'a self,
        snapshot: &'a str,
    ) -> BoxFuture<'a, Result<(), ProviderError>> {
        Box::pin(async move {
            self.call(Method::DELETE, &format!("/images/{snapshot}"), None)
                .await
                .map(|_| ())
        })
    }
}

/// Emplacements de `GET /locations`.
fn locations(body: &Value) -> Vec<Location> {
    let Some(list) = body["locations"].as_array() else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|l| {
            Some(Location {
                name: l["name"].as_str()?.to_owned(),
                city: l["city"].as_str()?.to_owned(),
                country: l["country"].as_str()?.to_owned(),
                latitude: l["latitude"].as_f64()?,
                longitude: l["longitude"].as_f64()?,
            })
        })
        .collect()
}

/// Machines d'une page de `GET /servers` qui portent l'étiquette `label`.
fn listed(body: &Value, label: &str) -> Vec<Listed> {
    let Some(servers) = body["servers"].as_array() else {
        return Vec::new();
    };
    servers
        .iter()
        .filter_map(|server| {
            Some(Listed {
                reference: server["id"].as_u64()?.to_string(),
                label: server["labels"][label].as_str()?.to_owned(),
                ipv4: server["public_net"]["ipv4"]["ip"]
                    .as_str()
                    .map(str::to_owned),
                created_at: OffsetDateTime::parse(server["created"].as_str()?, &Rfc3339).ok()?,
            })
        })
        .collect()
}

/// Types x86 commandables dans `location`, à son prix. Un prix par emplacement ne dit pas qu'on
/// peut y commander (refus `unsupported location for server type`) : la disponibilité se lit
/// dans `server_types[].locations[]` (`available`, `deprecation` par emplacement), seule source
/// depuis que `/datacenters` répond 410 (octobre 2026).
fn offers(body: &Value, location: &str) -> Vec<Offer> {
    let Some(types) = body["server_types"].as_array() else {
        return Vec::new();
    };
    let orderable = |t: &Value| {
        t["locations"].as_array().is_some_and(|locations| {
            locations.iter().any(|l| {
                l["name"] == location && l["available"] == true && l["deprecation"].is_null()
            })
        })
    };
    let mut offers: Vec<Offer> = types
        .iter()
        .filter(|t| t["architecture"] == "x86" && orderable(t))
        .filter_map(|t| {
            let price = t["prices"]
                .as_array()?
                .iter()
                .find(|p| p["location"] == location)?;
            Some(Offer {
                name: t["name"].as_str()?.to_owned(),
                vcpus: u32::try_from(t["cores"].as_u64()?).ok()?,
                memory_mb: (t["memory"].as_f64()? * 1024.0).round() as u32,
                disk_gb: u32::try_from(t["disk"].as_u64()?).ok()?,
                hourly_cost: decimal(price["price_hourly"]["net"].as_str()?, 5)?,
                monthly_cost: decimal(price["price_monthly"]["net"].as_str()?, 2)?,
            })
        })
        .collect();
    offers.sort_by_key(|o| o.hourly_cost);
    offers
}

/// `"3.7900000000"` en unités de 10^-`digits`, arrondi au-dessus (un coût n'est jamais sous-estimé).
fn decimal(text: &str, digits: usize) -> Option<i64> {
    let (int, frac) = text.split_once('.').unwrap_or((text, ""));
    if !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let kept = format!("{frac:0<digits$}");
    let (kept, rest) = kept.split_at(digits);
    let value = int.parse::<i64>().ok()? * 10_i64.pow(u32::try_from(digits).ok()?)
        + kept.parse::<i64>().unwrap_or(0);
    Some(if rest.bytes().any(|b| b != b'0') {
        value + 1
    } else {
        value
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locations_keep_their_city_and_coordinates() {
        let body = json!({ "locations": [
            { "name": "fsn1", "city": "Falkenstein", "country": "DE", "latitude": 50.47612, "longitude": 12.370071 },
            { "name": "broken" },
        ]});
        let found = locations(&body);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "fsn1");
        assert_eq!(found[0].city, "Falkenstein");
        assert_eq!(found[0].country, "DE");
    }

    #[test]
    fn decimal_rounds_costs_up() {
        assert_eq!(decimal("3.7900000000", 2), Some(379));
        assert_eq!(decimal("0.0060000000", 5), Some(600));
        assert_eq!(decimal("0.0060100000", 5), Some(601));
        assert_eq!(decimal("0.000001", 5), Some(1));
        assert_eq!(decimal("5", 2), Some(500));
        assert_eq!(decimal("1.x", 2), None);
    }

    #[test]
    fn offers_keep_x86_types_orderable_in_the_location() {
        let at = |name: &str, available: bool, deprecation: Value| json!({ "name": name, "available": available, "deprecation": deprecation });
        let price = |location: &str, hourly: &str, monthly: &str| {
            json!({ "location": location, "price_hourly": { "net": hourly },
                    "price_monthly": { "net": monthly } })
        };
        let body = json!({ "server_types": [
            { "name": "cpx31", "cores": 4, "memory": 8.0, "disk": 160, "architecture": "x86",
              "locations": [at("fsn1", true, Value::Null)],
              "prices": [price("fsn1", "0.0200000000", "12.4900000000")] },
            { "name": "cx23", "cores": 2, "memory": 4.0, "disk": 40, "architecture": "x86",
              "locations": [at("fsn1", true, Value::Null), at("ash", true, Value::Null)],
              "prices": [price("fsn1", "0.0060000000", "3.7900000000"),
                         price("ash", "0.0070000000", "4.5000000000")] },
            // Prix affiché, plus commandable à fsn1 (le refus vu en vrai).
            { "name": "cpx11", "cores": 2, "memory": 2.0, "disk": 40, "architecture": "x86",
              "locations": [at("fsn1", false, Value::Null)],
              "prices": [price("fsn1", "0.0088000000", "5.4900000000")] },
            // Déprécié dans l'emplacement.
            { "name": "cx22", "cores": 2, "memory": 4.0, "disk": 40, "architecture": "x86",
              "locations": [at("fsn1", true, json!({ "announced": "2026-01-01T00:00:00+00:00" }))],
              "prices": [price("fsn1", "0.0050000000", "3.2900000000")] },
            { "name": "cax11", "cores": 2, "memory": 4.0, "disk": 40, "architecture": "arm",
              "locations": [at("fsn1", true, Value::Null)],
              "prices": [price("fsn1", "0.0050000000", "3.2900000000")] },
            // Sans `locations` : jamais proposé.
            { "name": "old", "cores": 1, "memory": 2.0, "disk": 20, "architecture": "x86",
              "prices": [price("fsn1", "0.0010000000", "0.9900000000")] },
        ]});
        let found = offers(&body, "fsn1");
        let names: Vec<&str> = found.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["cx23", "cpx31"]);
        assert_eq!(found[0].memory_mb, 4096);
        assert_eq!(found[0].hourly_cost, 600);
        assert_eq!(found[0].monthly_cost, 379);
        assert!(offers(&body, "hel1").is_empty());
    }

    #[test]
    fn listed_servers_keep_their_label_and_creation_time() {
        let body = json!({ "servers": [
            { "id": 42, "labels": { "kuro-machine": "0190" }, "created": "2026-11-06T10:00:00+00:00",
              "public_net": { "ipv4": { "ip": "203.0.113.9" } } },
            { "id": 43, "labels": {}, "created": "2026-11-06T10:00:00+00:00", "public_net": {} },
        ]});
        let found = listed(&body, "kuro-machine");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].reference, "42");
        assert_eq!(found[0].label, "0190");
        assert_eq!(found[0].ipv4.as_deref(), Some("203.0.113.9"));
        assert_eq!(found[0].created_at.unix_timestamp(), 1_793_959_200);
    }
}
