//! `kuro-api setup` : première mise en route sans panel (ADR-0045), depuis l'hôte de l'API.
//!
//! Même cas d'usage que `POST /api/setup`, sans jeton : l'accès au shell vaut tous les droits.
//! Compte super admin et nom du projet, rien de plus ; le reste se règle dans le panel.

use crate::audit::domain::AuditContext;
use crate::config::Config;
use crate::iam::application::IamState;
use crate::setup::{SetupError, SetupRequest, SetupState};
use anyhow::Context;
use clap::Args;

use super::admin::PasswordSource;

#[derive(Debug, Args)]
pub struct Command {
    /// Adresse e-mail du super admin.
    #[arg(long)]
    email: String,
    /// Nom affiché du super admin.
    #[arg(long)]
    name: String,
    /// Nom du projet (1 à 80 caractères) ; absent : « Kuro ».
    #[arg(long)]
    project_name: Option<String>,
    #[command(flatten)]
    password: PasswordSource,
}

pub async fn run(command: Command, config: &Config) -> anyhow::Result<()> {
    let pool = super::connect(config).await?;
    let state = SetupState {
        iam: IamState::new(pool.clone(), &config.auth)?,
        pool,
        token_file: config.setup.token_file.clone(),
    };
    if !state.status().await?.needed {
        anyhow::bail!("setup already done: an admin account exists (use `admin create` for more)");
    }
    let password = command.password.read()?;
    let user = state
        .run(
            None,
            &AuditContext::cli("setup"),
            SetupRequest {
                email: command.email,
                display_name: command.name,
                password,
                project_name: command.project_name.clone(),
            },
        )
        .await
        .map_err(|error| match error {
            SetupError::Invalid(field) => {
                anyhow::anyhow!("invalid {}: {}", field.field, field.code.as_str())
            }
            other => anyhow::Error::from(other),
        })
        .context("setup")?;
    println!("super-admin {} created ({})", user.email, user.id);
    if let Some(name) = command.project_name {
        println!("project name: {name}");
    }
    if config.auth.require_totp {
        println!("TOTP will be configured at first login.");
    }
    println!("Setup done: sign in to the panel with this account.");
    Ok(())
}
