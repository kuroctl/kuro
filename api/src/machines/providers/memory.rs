//! Fournisseur simulé (tests) : deux offres fixes, VM gardées en mémoire avec leurs étiquettes.
//! `fail` fait refuser la prochaine création ; `lose_answer` crée la VM puis perd la réponse
//! (plantage entre le fournisseur et la base). Snapshots prêts aussitôt, sauf `snapshots_pending`.

use std::sync::{Arc, Mutex};

use futures_util::future::BoxFuture;
use time::OffsetDateTime;

use crate::machines::provider::{
    Listed, Location, Machine, MachineSpec, Offer, Provider, ProviderError, SnapshotState,
};

#[derive(Debug, Default)]
pub struct MemoryProvider {
    pub created: Mutex<Vec<MachineSpec>>,
    pub deleted: Mutex<Vec<String>>,
    pub fail: Mutex<Option<String>>,
    pub lose_answer: Mutex<bool>,
    pub snapshots_pending: Mutex<bool>,
    /// Snapshots : (référence, prêt).
    snapshots: Mutex<Vec<(String, bool)>>,
    vms: Mutex<Vec<(Listed, BTreeLabels)>>,
    counter: Mutex<u64>,
}

type BTreeLabels = std::collections::BTreeMap<String, String>;

impl MemoryProvider {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn created(&self) -> Vec<MachineSpec> {
        self.created.lock().map(|c| c.clone()).unwrap_or_default()
    }

    pub fn deleted(&self) -> Vec<String> {
        self.deleted.lock().map(|d| d.clone()).unwrap_or_default()
    }

    /// VM encore louées.
    pub fn vms(&self) -> Vec<Listed> {
        self.vms
            .lock()
            .map(|vms| vms.iter().map(|(vm, _)| vm.clone()).collect())
            .unwrap_or_default()
    }

    /// VM posée à la main (orpheline, création d'un autre passage), avec ses étiquettes ;
    /// `value` est celle de `label`. Sa référence.
    pub fn add_vm(
        &self,
        labels: &[(&str, &str)],
        label: &str,
        created_at: OffsetDateTime,
    ) -> String {
        let reference = self.next();
        let value = labels
            .iter()
            .find(|(key, _)| *key == label)
            .map_or("", |(_, value)| *value);
        let labels: BTreeLabels = labels
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        if let Ok(mut vms) = self.vms.lock() {
            vms.push((
                Listed {
                    reference: reference.clone(),
                    label: value.to_owned(),
                    ipv4: None,
                    created_at,
                },
                labels,
            ));
        }
        reference
    }

    fn next(&self) -> String {
        self.counter
            .lock()
            .map(|mut c| {
                *c += 1;
                c.to_string()
            })
            .unwrap_or_default()
    }
}

impl Provider for MemoryProvider {
    fn locations(&self) -> BoxFuture<'_, Result<Vec<Location>, ProviderError>> {
        let at = |name: &str, city: &str, country: &str, latitude, longitude| Location {
            name: name.to_owned(),
            city: city.to_owned(),
            country: country.to_owned(),
            latitude,
            longitude,
        };
        Box::pin(async move {
            Ok(vec![
                at("fsn1", "Falkenstein", "DE", 50.476, 12.370),
                at("nbg1", "Nuremberg", "DE", 49.452, 11.077),
                at("ash", "Ashburn, VA", "US", 39.045, -77.487),
            ])
        })
    }

    fn offers<'a>(
        &'a self,
        _location: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Offer>, ProviderError>> {
        Box::pin(async move {
            Ok(vec![
                Offer {
                    name: "small".to_owned(),
                    vcpus: 2,
                    memory_mb: 4096,
                    disk_gb: 40,
                    hourly_cost: 600,
                    monthly_cost: 379,
                },
                Offer {
                    name: "large".to_owned(),
                    vcpus: 4,
                    memory_mb: 8192,
                    disk_gb: 160,
                    hourly_cost: 2000,
                    monthly_cost: 1249,
                },
                // Trop petite pour un serveur une fois la réserve du système déduite.
                Offer {
                    name: "tiny".to_owned(),
                    vcpus: 1,
                    memory_mb: 1024,
                    disk_gb: 20,
                    hourly_cost: 300,
                    monthly_cost: 199,
                },
            ])
        })
    }

    fn create(&self, spec: MachineSpec) -> BoxFuture<'_, Result<Machine, ProviderError>> {
        Box::pin(async move {
            if let Some(message) = self.fail.lock().ok().and_then(|mut f| f.take()) {
                return Err(ProviderError::Refused {
                    code: "resource_limit_exceeded".to_owned(),
                    message,
                });
            }
            let reference = self.next();
            let ipv4 = Some(format!("203.0.113.{reference}"));
            if let Ok(mut vms) = self.vms.lock() {
                let label = spec
                    .labels
                    .get(crate::machines::domain::LABEL)
                    .cloned()
                    .unwrap_or_default();
                vms.push((
                    Listed {
                        reference: reference.clone(),
                        label,
                        ipv4: ipv4.clone(),
                        created_at: OffsetDateTime::now_utc(),
                    },
                    spec.labels.clone(),
                ));
            }
            if let Ok(mut created) = self.created.lock() {
                created.push(spec);
            }
            if self
                .lose_answer
                .lock()
                .is_ok_and(|mut lose| std::mem::take(&mut *lose))
            {
                return Err(ProviderError::Transport("connection reset".to_owned()));
            }
            Ok(Machine { reference, ipv4 })
        })
    }

    fn list<'a>(
        &'a self,
        label: &'a str,
        scope: (&'a str, &'a str),
    ) -> BoxFuture<'a, Result<Vec<Listed>, ProviderError>> {
        Box::pin(async move {
            Ok(self
                .vms
                .lock()
                .map(|vms| {
                    vms.iter()
                        .filter(|(_, labels)| {
                            labels.contains_key(label)
                                && labels.get(scope.0).map(String::as_str) == Some(scope.1)
                        })
                        .map(|(vm, _)| vm.clone())
                        .collect()
                })
                .unwrap_or_default())
        })
    }

    fn snapshot<'a>(
        &'a self,
        _reference: &'a str,
        _labels: &'a std::collections::BTreeMap<String, String>,
    ) -> BoxFuture<'a, Result<String, ProviderError>> {
        Box::pin(async move {
            let reference = format!("snap-{}", self.next());
            let ready = !self.snapshots_pending.lock().is_ok_and(|p| *p);
            if let Ok(mut snapshots) = self.snapshots.lock() {
                snapshots.push((reference.clone(), ready));
            }
            Ok(reference)
        })
    }

    fn snapshot_state<'a>(
        &'a self,
        snapshot: &'a str,
    ) -> BoxFuture<'a, Result<SnapshotState, ProviderError>> {
        Box::pin(async move {
            let found = self.snapshots.lock().ok().and_then(|s| {
                s.iter()
                    .find(|(r, _)| r == snapshot)
                    .map(|(_, ready)| *ready)
            });
            Ok(match found {
                Some(true) => SnapshotState::Ready,
                Some(false) => SnapshotState::Pending,
                None => SnapshotState::Missing,
            })
        })
    }

    fn delete_snapshot<'a>(
        &'a self,
        snapshot: &'a str,
    ) -> BoxFuture<'a, Result<(), ProviderError>> {
        Box::pin(async move {
            if let Ok(mut snapshots) = self.snapshots.lock() {
                snapshots.retain(|(r, _)| r != snapshot);
            }
            Ok(())
        })
    }

    fn delete<'a>(&'a self, reference: &'a str) -> BoxFuture<'a, Result<(), ProviderError>> {
        Box::pin(async move {
            // Déjà absente : succès, comme le 404 d'un vrai fournisseur.
            if let Ok(mut vms) = self.vms.lock() {
                vms.retain(|(vm, _)| vm.reference != reference);
            }
            if let Ok(mut deleted) = self.deleted.lock() {
                deleted.push(reference.to_owned());
            }
            Ok(())
        })
    }
}

impl MemoryProvider {
    /// Snapshots encore gardés.
    pub fn snapshots(&self) -> Vec<String> {
        self.snapshots
            .lock()
            .map(|s| s.iter().map(|(r, _)| r.clone()).collect())
            .unwrap_or_default()
    }

    /// Snapshots en cours terminés.
    pub fn finish_snapshots(&self) {
        if let Ok(mut snapshots) = self.snapshots.lock() {
            for snapshot in snapshots.iter_mut() {
                snapshot.1 = true;
            }
        }
    }

    /// VM disparue chez le fournisseur sans passer par `delete` (tests).
    pub fn delete_now(&self, reference: &str) {
        if let Ok(mut vms) = self.vms.lock() {
            vms.retain(|(vm, _)| vm.reference != reference);
        }
    }
}
