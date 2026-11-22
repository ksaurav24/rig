//! `rig tokens [ls|create|revoke]`.

use clap::{Args, Subcommand, ValueEnum};
use rig_api::{CreateApiTokenRequest, Role};
use serde_json::json;

use crate::api::Client;
use crate::error::{CliError, CliResult};
use crate::output::{
    OutputOpts, confirm, label, print_names, print_structured, relative_time, short, table,
};

#[derive(Args)]
pub struct TokensArgs {
    #[command(subcommand)]
    pub command: Option<TokensCommand>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum RoleArg {
    Viewer,
    Developer,
    Admin,
    Owner,
}

impl From<RoleArg> for Role {
    fn from(v: RoleArg) -> Self {
        match v {
            RoleArg::Viewer => Role::Viewer,
            RoleArg::Developer => Role::Developer,
            RoleArg::Admin => Role::Admin,
            RoleArg::Owner => Role::Owner,
        }
    }
}

#[derive(Subcommand)]
pub enum TokensCommand {
    /// List your API tokens.
    Ls,
    /// Create a new API token (the secret is shown once).
    Create {
        /// Human label for the token (e.g. `ci-deploy`).
        name: String,
        /// Permission role for the token.
        #[arg(long, value_enum, default_value_t = RoleArg::Developer)]
        role: RoleArg,
        /// Optional lifetime in days (1–3650). Omit for a non-expiring token.
        #[arg(long)]
        expires_days: Option<u32>,
    },
    /// Revoke an API token by id, name, or prefix.
    Revoke {
        /// Token id, name, or prefix.
        token: String,
    },
}

pub async fn run(api: &Client, args: TokensArgs, out: OutputOpts) -> CliResult<()> {
    match args.command {
        None | Some(TokensCommand::Ls) => {
            let tokens = api.list_tokens().await?;
            if out.is_structured() {
                return print_structured(&tokens, out.format);
            }
            if out.is_name() {
                let names: Vec<String> = tokens.iter().map(|t| t.id.clone()).collect();
                print_names(&names);
                return Ok(());
            }
            let rows = tokens
                .iter()
                .map(|t| {
                    vec![
                        short(&t.id),
                        t.name.clone(),
                        t.prefix.clone(),
                        label(&t.role),
                        t.last_used_at
                            .as_deref()
                            .map(relative_time)
                            .unwrap_or_else(|| "never".into()),
                        t.expires_at
                            .as_deref()
                            .map(|s| s.chars().take(10).collect())
                            .unwrap_or_else(|| "never".into()),
                    ]
                })
                .collect();
            table(
                &["ID", "NAME", "PREFIX", "ROLE", "LAST USED", "EXPIRES"],
                rows,
                Some("No API tokens yet. Create one with: rig tokens create <name>"),
                out.wide,
            );
        }
        Some(TokensCommand::Create {
            name,
            role,
            expires_days,
        }) => {
            let created = api
                .create_token(&CreateApiTokenRequest {
                    name,
                    role: role.into(),
                    expires_in_days: expires_days,
                })
                .await?;
            if out.is_structured() {
                return print_structured(&created, out.format);
            }
            if out.is_name() {
                println!("{}", created.secret);
                return Ok(());
            }
            println!(
                "Created token {} ({}, {})",
                created.token.name,
                label(&created.token.role),
                created.token.id
            );
            println!("Secret (shown once): {}", created.secret);
        }
        Some(TokensCommand::Revoke { token }) => {
            let tokens = api.list_tokens().await?;
            let needle = token.trim();
            let matches: Vec<_> = tokens
                .into_iter()
                .filter(|t| {
                    t.id == needle
                        || t.name.eq_ignore_ascii_case(needle)
                        || t.prefix == needle
                        || (needle.len() >= 4 && t.id.ends_with(needle))
                })
                .collect();
            let target = match matches.len() {
                0 => {
                    return Err(CliError::not_found(format!(
                        "no token matching `{needle}`. List them with: rig tokens ls"
                    )));
                }
                1 => matches.into_iter().next().unwrap(),
                _ => {
                    return Err(CliError::message(format!(
                        "`{needle}` matches multiple tokens; pass the full token id"
                    )));
                }
            };
            confirm(
                &format!("Revoke API token {} ({})?", target.name, target.prefix),
                out.yes,
            )?;
            api.delete_token(&target.id).await?;
            if out.is_structured() {
                return print_structured(
                    &json!({ "revoked": target.id, "name": target.name }),
                    out.format,
                );
            }
            if out.is_name() {
                println!("{}", target.id);
                return Ok(());
            }
            println!("Revoked token {} ({})", target.name, short(&target.id));
        }
    }
    Ok(())
}
