//! `rig env ls|set|get|rm`.

use clap::{Args, Subcommand};
use serde_json::json;

use crate::api::Client;
use crate::error::CliResult;
use crate::ops;
use crate::output::{print_json, table};

#[derive(Args)]
pub struct EnvArgs {
    #[command(subcommand)]
    pub command: EnvCommand,
}

#[derive(Subcommand)]
pub enum EnvCommand {
    /// List variables. Secret values are hidden.
    Ls { app: String },
    /// Set variables: `rig env set web KEY=value OTHER=value`. Stored as secrets unless --plain.
    Set {
        app: String,
        #[arg(required = true, value_name = "KEY=VALUE")]
        pairs: Vec<String>,
        /// Store as a plain, readable value instead of a secret.
        #[arg(long)]
        plain: bool,
    },
    /// Print one variable's value. Reading a secret is recorded in the audit log.
    Get { app: String, key: String },
    /// Delete a variable.
    Rm { app: String, key: String },
}

pub async fn run(api: &Client, args: EnvArgs, json_out: bool, wide: bool) -> CliResult<()> {
    match args.command {
        EnvCommand::Ls { app } => {
            let app = ops::apps::resolve_app(api, &app).await?;
            let vars = api.list_env(&app.id).await?;
            if json_out {
                return print_json(&vars);
            }
            let rows = vars
                .iter()
                .map(|v| {
                    vec![
                        v.key.clone(),
                        v.value.clone().unwrap_or_else(|| "(secret)".into()),
                    ]
                })
                .collect();
            table(
                &["KEY", "VALUE"],
                rows,
                Some(&format!(
                    "No environment variables on {}. Set one with: rig env set {} KEY=value",
                    app.name, app.name
                )),
                wide,
            );
        }
        EnvCommand::Set { app, pairs, plain } => {
            let parsed = ops::env::parse_pairs(&pairs)?;
            let app = ops::apps::resolve_app(api, &app).await?;
            let saved = ops::env::set_batch(api, &app.id, &parsed, !plain).await?;
            if json_out {
                return print_json(&json!({
                    "app": app.name,
                    "set": saved,
                }));
            }
            for var in &saved {
                println!("Set {}", var.key);
            }
            println!("Changes apply on the next deploy: rig deploy {}", app.name);
        }
        EnvCommand::Get { app, key } => {
            let app = ops::apps::resolve_app(api, &app).await?;
            let var = api.reveal_env(&app.id, &key).await?;
            if json_out {
                return print_json(&var);
            }
            println!("{}", var.value);
        }
        EnvCommand::Rm { app, key } => {
            let app = ops::apps::resolve_app(api, &app).await?;
            api.delete_env(&app.id, &key).await?;
            if json_out {
                return print_json(&json!({
                    "app": app.name,
                    "deleted": key,
                }));
            }
            println!("Deleted {key}");
        }
    }
    Ok(())
}
