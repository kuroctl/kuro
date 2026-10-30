//! `kuro-api admin …` : premier compte et récupération d'accès, depuis l'hôte de l'API.
//!
//! Mêmes cas d'usage que le REST, avec l'autorité `Host` et un contexte d'audit `cli`.
//! Les mots de passe sont saisis sans écho ou lus sur l'entrée standard, jamais en argument.

use std::io::{BufRead, IsTerminal};

use anyhow::{Context, bail};
use clap::{Args, Subcommand};
use kuro_api::audit::domain::AuditContext;
use kuro_api::config::Config;
use kuro_api::iam::application::{IamState, NewAdminUser, users};
use kuro_api::iam::domain::{Authority, SUPER_ADMIN_ROLE_ID};

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Crée un compte super-admin (premier accès ou secours).
    Create {
        #[arg(long)]
        email: String,
        #[arg(long)]
        name: String,
        #[command(flatten)]
        password: PasswordSource,
    },
    /// Impose un nouveau mot de passe, déverrouille le compte et ferme ses sessions.
    ResetPassword {
        #[arg(long)]
        email: String,
        #[command(flatten)]
        password: PasswordSource,
    },
    /// Réinitialise le TOTP (téléphone perdu) : à reconfigurer à la prochaine connexion.
    ResetTotp {
        #[arg(long)]
        email: String,
    },
}

#[derive(Debug, Args)]
pub struct PasswordSource {
    /// Lit le mot de passe sur l'entrée standard (scripts) au lieu de le demander.
    #[arg(long)]
    password_stdin: bool,
}

pub async fn run(command: Command, config: &Config) -> anyhow::Result<()> {
    let state = IamState::new(super::connect(config).await?, &config.auth)?;
    match command {
        Command::Create {
            email,
            name,
            password,
        } => {
            let password = password.read()?;
            let context = AuditContext::cli("admin create");
            let user = users::create(
                &state,
                &Authority::Host,
                &context,
                NewAdminUser {
                    email,
                    display_name: name,
                    password,
                },
            )
            .await?;
            users::set_roles(
                &state,
                &Authority::Host,
                &context,
                user.id,
                &[SUPER_ADMIN_ROLE_ID],
            )
            .await
            .with_context(|| {
                format!(
                    "account {} created but super-admin role not granted",
                    user.id
                )
            })?;
            println!("super-admin {} created ({})", user.email, user.id);
            if config.auth.require_totp {
                println!("TOTP will be configured at first login.");
            }
        }
        Command::ResetPassword { email, password } => {
            let password = password.read()?;
            let user = users::find_by_email(&state, &email).await?;
            let context = AuditContext::cli("admin reset-password");
            users::reset_password(&state, &Authority::Host, &context, user.id, &password).await?;
            println!("password reset for {}, sessions closed", user.email);
        }
        Command::ResetTotp { email } => {
            let user = users::find_by_email(&state, &email).await?;
            let context = AuditContext::cli("admin reset-totp");
            users::reset_totp(&state, &Authority::Host, &context, user.id).await?;
            println!("TOTP reset for {}, sessions closed", user.email);
        }
    }
    Ok(())
}

impl PasswordSource {
    fn read(&self) -> anyhow::Result<String> {
        if self.password_stdin {
            let mut line = String::new();
            std::io::stdin().lock().read_line(&mut line)?;
            return Ok(line.trim_end_matches(['\r', '\n']).to_owned());
        }
        if !std::io::stdin().is_terminal() {
            bail!("no terminal to prompt for the password: use --password-stdin");
        }
        let password = rpassword::prompt_password("Password: ")?;
        if rpassword::prompt_password("Confirm password: ")? != password {
            bail!("passwords do not match");
        }
        Ok(password)
    }
}
