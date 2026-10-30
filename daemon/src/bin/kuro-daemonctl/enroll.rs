//! `kuro-daemonctl enroll`

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use clap::Args;
use kuro_daemon::enroll::{EnrollOptions, TOKEN_ENV, enroll};

#[derive(Debug, Args)]
pub struct EnrollArgs {
    /// Listener daemon de l'API, par exemple `https://panel.example.com:8443`.
    #[arg(long)]
    api: String,
    /// Empreinte de la CA affichée par `kuro-api node add` (`sha256:…`).
    #[arg(long)]
    ca_fingerprint: String,
    /// Fichier contenant le token. À défaut, le token est lu dans `KURO_ENROLL_TOKEN`.
    /// Il n'existe volontairement pas d'option `--token` : il finirait dans l'historique du shell.
    #[arg(long)]
    token_file: Option<PathBuf>,
}

pub fn run(args: EnrollArgs, config_path: &Path) -> anyhow::Result<()> {
    let token = read_token(args.token_file.as_deref())?;
    let options = EnrollOptions {
        api_url: args.api,
        ca_fingerprint: args.ca_fingerprint,
        token,
        config_path: config_path.to_owned(),
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let enrolled = runtime.block_on(enroll(&options))?;

    println!(
        "node {} enrolled ({})",
        enrolled.node_name, enrolled.node_id
    );
    println!(
        "configuration written to {}",
        enrolled.config_path.display()
    );
    Ok(())
}

fn read_token(token_file: Option<&Path>) -> anyhow::Result<String> {
    let token = match token_file {
        Some(path) => std::fs::read_to_string(path)
            .with_context(|| format!("reading token file {}", path.display()))?,
        None => std::env::var(TOKEN_ENV).unwrap_or_default(),
    };
    let token = token.trim().to_owned();
    if token.is_empty() {
        bail!("no enrollment token: set {TOKEN_ENV} or pass --token-file");
    }
    Ok(token)
}
