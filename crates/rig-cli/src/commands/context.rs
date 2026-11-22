//! `rig context`, `rig context use`, `rig context current`, and `rig context rename`.

use clap::{Args, Subcommand};
use serde_json::json;

use crate::config::Config;
use crate::error::{CliError, CliResult};
use crate::output::{OutputOpts, print_names, print_structured, table};

#[derive(Args)]
pub struct ContextArgs {
    #[command(subcommand)]
    pub command: Option<ContextCommand>,
}

#[derive(Subcommand)]
pub enum ContextCommand {
    /// List saved contexts.
    Ls,
    /// Switch the current context.
    Use { name: String },
    /// Print the active context name.
    Current,
    /// Rename a saved context.
    Rename { old: String, new_name: String },
}

pub fn run(config: &mut Config, args: ContextArgs, out: OutputOpts) -> CliResult<()> {
    match args.command {
        None | Some(ContextCommand::Ls) => {
            if out.is_structured() {
                let items: Vec<_> = config
                    .contexts
                    .iter()
                    .map(|(name, ctx)| {
                        json!({
                            "name": name,
                            "url": ctx.url,
                            "user": ctx.user,
                            "org": ctx.org,
                            "role": ctx.role,
                            "current": config.current.as_deref() == Some(name.as_str()),
                        })
                    })
                    .collect();
                return print_structured(&items, out.format);
            }
            if out.is_name() {
                let names: Vec<String> = config.contexts.keys().cloned().collect();
                print_names(&names);
                return Ok(());
            }
            let has_meta = config
                .contexts
                .values()
                .any(|c| c.user.is_some() || c.org.is_some());
            if has_meta {
                let rows = config
                    .contexts
                    .iter()
                    .map(|(name, ctx)| {
                        let mark = if config.current.as_deref() == Some(name.as_str()) {
                            "*"
                        } else {
                            ""
                        };
                        vec![
                            mark.to_string(),
                            name.clone(),
                            ctx.url.clone(),
                            ctx.user.clone().unwrap_or_default(),
                            ctx.org.clone().unwrap_or_default(),
                        ]
                    })
                    .collect();
                table(
                    &["", "NAME", "SERVER", "USER", "ORG"],
                    rows,
                    Some("No contexts. Add one with: rig login <url>"),
                    out.wide,
                );
            } else {
                let rows = config
                    .contexts
                    .iter()
                    .map(|(name, ctx)| {
                        let mark = if config.current.as_deref() == Some(name.as_str()) {
                            "*"
                        } else {
                            ""
                        };
                        vec![mark.to_string(), name.clone(), ctx.url.clone()]
                    })
                    .collect();
                table(
                    &["", "NAME", "SERVER"],
                    rows,
                    Some("No contexts. Add one with: rig login <url>"),
                    out.wide,
                );
            }
            Ok(())
        }
        Some(ContextCommand::Use { name }) => {
            config.select(&name)?;
            config.save()?;
            if out.is_structured() {
                return print_structured(&json!({ "current": name }), out.format);
            }
            if out.is_name() {
                println!("{name}");
                return Ok(());
            }
            println!("Now using {name}");
            Ok(())
        }
        Some(ContextCommand::Current) => {
            let Some(name) = config.current.as_deref() else {
                return Err(CliError::auth("not signed in. Run: rig login <url>"));
            };
            let ctx = config.contexts.get(name);
            if out.is_structured() {
                return print_structured(
                    &json!({
                        "name": name,
                        "url": ctx.map(|c| c.url.as_str()),
                        "user": ctx.and_then(|c| c.user.as_deref()),
                        "org": ctx.and_then(|c| c.org.as_deref()),
                        "role": ctx.and_then(|c| c.role.as_deref()),
                    }),
                    out.format,
                );
            }
            println!("{name}");
            Ok(())
        }
        Some(ContextCommand::Rename { old, new_name }) => {
            config.rename(&old, &new_name)?;
            config.save()?;
            if out.is_structured() {
                return print_structured(
                    &json!({ "from": old, "to": new_name, "current": config.current }),
                    out.format,
                );
            }
            if out.is_name() {
                println!("{new_name}");
                return Ok(());
            }
            println!("Renamed context {old} → {new_name}");
            Ok(())
        }
    }
}
