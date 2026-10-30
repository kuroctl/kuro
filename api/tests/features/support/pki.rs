//! PKI de test : initialisée par le vrai cas d'usage `pki init`, dans un dossier temporaire.

use std::path::{Path, PathBuf};

use kuro_api::pki::application;
use kuro_api::pki::domain::CertificateAuthority;
use kuro_api::pki::infra::PkiFiles;
use rcgen::{CertificateParams, DnType, KeyPair};
use tempfile::TempDir;

pub struct TestPki {
    dir: TempDir,
    pub files: PkiFiles,
    pub fingerprint: String,
}

/// Certificat client, clé et CA de confiance d'un nœud, sur disque.
pub struct NodeFiles {
    pub ca_cert: PathBuf,
    pub cert: PathBuf,
    pub key: PathBuf,
}

impl TestPki {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let files = PkiFiles {
            ca_cert: dir.path().join("ca.pem"),
            ca_key: dir.path().join("ca.key"),
            server_cert: dir.path().join("api.pem"),
            server_key: dir.path().join("api.key"),
        };
        let names = ["localhost".to_owned(), "127.0.0.1".to_owned()];
        let report = application::init(&files, &names).unwrap();
        Self {
            dir,
            files,
            fingerprint: report.ca_fingerprint,
        }
    }

    pub fn ca(&self) -> CertificateAuthority {
        self.files.load_ca().unwrap()
    }

    /// Certificat signé par la CA de l'API pour un nœud absent de la base.
    pub fn issue_unregistered(&self, name: &str) -> NodeFiles {
        self.issue_with(&self.ca(), name)
    }

    /// Certificat valide en apparence, mais signé par une autre CA.
    pub fn issue_foreign(&self, name: &str) -> NodeFiles {
        self.issue_with(&CertificateAuthority::generate("Rogue CA").unwrap(), name)
    }

    fn issue_with(&self, ca: &CertificateAuthority, name: &str) -> NodeFiles {
        let (key, csr) = key_and_csr("node");
        let signed = ca
            .sign_node_csr(&csr, name, time::Duration::days(1))
            .unwrap();
        NodeFiles {
            ca_cert: self.files.ca_cert.clone(),
            cert: write(self.dir.path(), &format!("{name}.pem"), &signed.cert_pem),
            key: write(
                self.dir.path(),
                &format!("{name}.key"),
                &key.serialize_pem(),
            ),
        }
    }
}

/// Clé et CSR telles que les produirait un hôte, avec le CN demandé.
pub fn key_and_csr(common_name: &str) -> (KeyPair, String) {
    let key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params
        .distinguished_name
        .push(DnType::CommonName, common_name);
    let csr = params.serialize_request(&key).unwrap().pem().unwrap();
    (key, csr)
}

fn write(dir: &Path, name: &str, content: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    path
}
