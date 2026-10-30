//! Fournisseur DNS simulé (ADR-0026) : une zone `kuroctl.test` ouverte par le jeton `good-token`,
//! enregistrements en mémoire.

use std::collections::BTreeMap;
use std::sync::Mutex;

use futures_util::future::BoxFuture;
use kuro_api::dns::domain::Desired;
use kuro_api::dns::provider::{DnsProvider, ProviderError, Remote, ZoneAccess};

pub const TOKEN: &str = "good-token";
pub const ZONE: &str = "zone-kuroctl";

#[derive(Debug, Default)]
pub struct FakeDns {
    pub records: Mutex<BTreeMap<String, Desired>>,
    next: Mutex<u32>,
    /// Refuse les créations (panne simulée).
    pub down: Mutex<bool>,
}

impl FakeDns {
    /// Enregistrements publiés, triés par nom puis type : `(type, nom, contenu)`.
    pub fn published(&self) -> Vec<(String, String, String)> {
        let mut out: Vec<_> = self
            .records
            .lock()
            .unwrap()
            .values()
            .map(|r| {
                (
                    r.kind.as_str().to_owned(),
                    r.name.clone(),
                    r.content.clone(),
                )
            })
            .collect();
        out.sort_by(|a, b| (&a.1, &a.0).cmp(&(&b.1, &b.0)));
        out
    }

    /// Supprime à la main, comme depuis le tableau de bord du fournisseur.
    pub fn remove_by_name(&self, name: &str) {
        self.records.lock().unwrap().retain(|_, r| r.name != name);
    }

    fn check(&self, access: &ZoneAccess) -> Result<(), ProviderError> {
        if access.token != TOKEN {
            return Err(ProviderError::Unauthorized);
        }
        if access.zone != ZONE {
            return Err(ProviderError::ZoneNotFound);
        }
        Ok(())
    }
}

impl DnsProvider for FakeDns {
    fn find_zone<'a>(
        &'a self,
        token: &'a str,
        domain: &'a str,
    ) -> BoxFuture<'a, Result<String, ProviderError>> {
        Box::pin(async move {
            if token != TOKEN {
                return Err(ProviderError::Unauthorized);
            }
            if domain == "kuroctl.test" || domain.ends_with(".kuroctl.test") {
                Ok(ZONE.to_owned())
            } else {
                Err(ProviderError::ZoneNotFound)
            }
        })
    }

    fn list<'a>(
        &'a self,
        access: &'a ZoneAccess,
    ) -> BoxFuture<'a, Result<Vec<Remote>, ProviderError>> {
        Box::pin(async move {
            self.check(access)?;
            Ok(self
                .records
                .lock()
                .unwrap()
                .iter()
                .map(|(id, record)| Remote {
                    id: id.clone(),
                    record: record.clone(),
                })
                .collect())
        })
    }

    fn create<'a>(
        &'a self,
        access: &'a ZoneAccess,
        record: &'a Desired,
    ) -> BoxFuture<'a, Result<String, ProviderError>> {
        Box::pin(async move {
            self.check(access)?;
            if *self.down.lock().unwrap() {
                return Err(ProviderError::Unavailable("down".to_owned()));
            }
            let mut next = self.next.lock().unwrap();
            *next += 1;
            let id = format!("rec-{next}");
            self.records
                .lock()
                .unwrap()
                .insert(id.clone(), record.clone());
            Ok(id)
        })
    }

    fn update<'a>(
        &'a self,
        access: &'a ZoneAccess,
        id: &'a str,
        record: &'a Desired,
    ) -> BoxFuture<'a, Result<(), ProviderError>> {
        Box::pin(async move {
            self.check(access)?;
            self.records
                .lock()
                .unwrap()
                .insert(id.to_owned(), record.clone());
            Ok(())
        })
    }

    fn delete<'a>(
        &'a self,
        access: &'a ZoneAccess,
        id: &'a str,
    ) -> BoxFuture<'a, Result<(), ProviderError>> {
        Box::pin(async move {
            self.check(access)?;
            self.records.lock().unwrap().remove(id);
            Ok(())
        })
    }
}
