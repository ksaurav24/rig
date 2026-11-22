//! `rig env ls|set|get|rm|import|pull|diff`.

use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;

use clap::{Args, Subcommand};
use serde_json::json;

use crate::api::Client;
use crate::error::{CliError, CliResult};
use crate::ops;
use crate::ops::env::DiffKind;
use crate::output::{OutputOpts, print_names, print_structured, table};

#[derive(Args)]
pub struct EnvArgs {
    #[command(subcommand)]
    pub command: EnvCommand,
}

#[derive(Subcommand)]
pub enum EnvCommand {
    /// List variables. Secret values are hidden.
    Ls {
        /// App name, `project/app`, or id. Defaults to `rig.toml`.
        app: Option<String>,
    },
    /// Set variables: `rig env set web KEY=value OTHER=-` (use `KEY=-` to read from stdin).
    Set {
        /// App name, `project/app`, or id.
        app: String,
        #[arg(required = true, value_name = "KEY=VALUE")]
        pairs: Vec<String>,
        /// Store as plain, readable values instead of secrets.
        #[arg(long)]
        plain: bool,
    },
    /// Print one variable's value. Reading a secret is recorded in the audit log.
    Get {
        /// App name, `project/app`, or id.
        app: String,
        key: String,
    },
    /// Delete a variable.
    Rm {
        /// App name, `project/app`, or id.
        app: String,
        key: String,
    },
    /// Atomically import variables from a `.env` file (or `-` for stdin).
    Import {
        /// App name, `project/app`, or id.
        app: String,
        /// Path to `.env` file, or `-` to read from stdin.
        file: String,
        /// Store imported values as plain text instead of secrets.
        #[arg(long)]
        plain: bool,
        /// Replace all existing variables on the app with only the ones in the file.
        #[arg(long)]
        replace: bool,
    },
    /// Export variables in `.env` format (to stdout or `--file`).
    Pull {
        /// App name, `project/app`, or id. Defaults to `rig.toml`.
        app: Option<String>,
        /// Write to this file instead of stdout.
        #[arg(short = 'f', long)]
        file: Option<PathBuf>,
        /// Reveal secret values (each reveal is recorded in the audit log).
        #[arg(long)]
        reveal: bool,
    },
    /// Compare a local `.env` file against an app's environment variables.
    Diff {
        /// App name, `project/app`, or id.
        app: String,
        /// Path to local `.env` file.
        file: PathBuf,
        /// Reveal secret values on the server to compare them accurately (audited).
        #[arg(long)]
        reveal: bool,
    },
}

pub async fn run(api: &Client, args: EnvArgs, out: OutputOpts) -> CliResult<()> {
    match args.command {
        EnvCommand::Ls { app } => {
            let app = ops::apps::resolve_app_or_linked(api, app.as_deref()).await?;
            let vars = api.list_env(&app.id).await?;
            if out.is_structured() {
                return print_structured(&vars, out.format);
            }
            if out.is_name() {
                let names: Vec<String> = vars.iter().map(|v| v.key.clone()).collect();
                print_names(&names);
                return Ok(());
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
                out.wide,
            );
        }
        EnvCommand::Set { app, pairs, plain } => {
            let parsed = ops::env::parse_pairs(&pairs)?;
            let app = ops::apps::resolve_app(api, &app).await?;
            let saved = ops::env::set_batch(api, &app.id, &parsed, !plain, false).await?;
            if out.is_structured() {
                return print_structured(
                    &json!({
                        "app": app.name,
                        "set": saved,
                    }),
                    out.format,
                );
            }
            if out.is_name() {
                let names: Vec<String> = saved.iter().map(|v| v.key.clone()).collect();
                print_names(&names);
                return Ok(());
            }
            for var in &saved {
                println!("Set {}", var.key);
            }
            println!("Changes apply on the next deploy: rig deploy {}", app.name);
        }
        EnvCommand::Get { app, key } => {
            let app = ops::apps::resolve_app(api, &app).await?;
            let var = api.reveal_env(&app.id, &key).await?;
            if out.is_structured() {
                return print_structured(&var, out.format);
            }
            println!("{}", var.value);
        }
        EnvCommand::Rm { app, key } => {
            let app = ops::apps::resolve_app(api, &app).await?;
            api.delete_env(&app.id, &key).await?;
            if out.is_structured() {
                return print_structured(
                    &json!({
                        "app": app.name,
                        "deleted": key,
                    }),
                    out.format,
                );
            }
            if out.is_name() {
                println!("{key}");
                return Ok(());
            }
            println!("Deleted {key}");
        }
        EnvCommand::Import {
            app,
            file,
            plain,
            replace,
        } => {
            let text = if file == "-" {
                let mut buf = String::new();
                std::io::stdin().read_to_string(&mut buf)?;
                buf
            } else {
                std::fs::read_to_string(&file)
                    .map_err(|e| CliError::message(format!("could not read {file}: {e}")))?
            };
            let pairs = ops::env::parse_dotenv(&text)?;
            if pairs.is_empty() && !replace {
                return Err(CliError::message("no variables found to import"));
            }
            let app = ops::apps::resolve_app(api, &app).await?;
            let saved = ops::env::set_batch(api, &app.id, &pairs, !plain, replace).await?;
            if out.is_structured() {
                return print_structured(
                    &json!({
                        "app": app.name,
                        "imported": saved.len(),
                        "replace": replace,
                        "vars": saved,
                    }),
                    out.format,
                );
            }
            if out.is_name() {
                let names: Vec<String> = saved.iter().map(|v| v.key.clone()).collect();
                print_names(&names);
                return Ok(());
            }
            println!("Imported {} variable(s) into {}", saved.len(), app.name);
            println!("Changes apply on the next deploy: rig deploy {}", app.name);
        }
        EnvCommand::Pull { app, file, reveal } => {
            let app = ops::apps::resolve_app_or_linked(api, app.as_deref()).await?;
            let vars = api.list_env(&app.id).await?;
            let mut revealed = HashMap::new();
            if reveal {
                for v in &vars {
                    if v.is_secret {
                        let rev = api.reveal_env(&app.id, &v.key).await?;
                        revealed.insert(rev.key, rev.value);
                    }
                }
            }
            if out.is_structured() {
                let items: Vec<_> = vars
                    .iter()
                    .map(|v| {
                        let val = v
                            .value
                            .clone()
                            .or_else(|| revealed.get(&v.key).cloned());
                        json!({
                            "key": v.key,
                            "value": val,
                            "is_secret": v.is_secret,
                        })
                    })
                    .collect();
                return print_structured(&items, out.format);
            }
            let content = ops::env::format_dotenv(&vars, &revealed);
            if let Some(path) = file {
                std::fs::write(&path, &content).map_err(|e| {
                    CliError::message(format!("could not write {}: {e}", path.display()))
                })?;
                println!("Wrote {} variable(s) to {}", vars.len(), path.display());
            } else {
                print!("{content}");
            }
        }
        EnvCommand::Diff { app, file, reveal } => {
            let text = std::fs::read_to_string(&file).map_err(|e| {
                CliError::message(format!("could not read {}: {e}", file.display()))
            })?;
            let local = ops::env::parse_dotenv(&text)?;
            let app = ops::apps::resolve_app(api, &app).await?;
            let remote = api.list_env(&app.id).await?;
            let mut revealed = HashMap::new();
            if reveal {
                for v in &remote {
                    if v.is_secret {
                        let rev = api.reveal_env(&app.id, &v.key).await?;
                        revealed.insert(rev.key, rev.value);
                    }
                }
            }
            let diffs = ops::env::diff_env(&local, &remote, &revealed);
            if out.is_structured() {
                return print_structured(&diffs, out.format);
            }
            if out.is_name() {
                let names: Vec<String> = diffs.iter().map(|d| d.key.clone()).collect();
                print_names(&names);
                return Ok(());
            }
            if diffs.is_empty() {
                println!("No differences between {} and {}", file.display(), app.name);
                return Ok(());
            }
            let rows = diffs
                .iter()
                .map(|d| {
                    let sym = match d.kind {
                        DiffKind::Added => "+ local only",
                        DiffKind::Removed => "- server only",
                        DiffKind::Changed => "~ changed",
                        DiffKind::SecretMasked => "? secret (pass --reveal)",
                    };
                    vec![
                        sym.to_string(),
                        d.key.clone(),
                        d.local.clone().unwrap_or_default(),
                        d.remote.clone().unwrap_or_default(),
                    ]
                })
                .collect();
            table(&["CHANGE", "KEY", "LOCAL", "SERVER"], rows, None, out.wide);
        }
    }
    Ok(())
}
