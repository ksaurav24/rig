//! `rig context` and `rig context use <name>`.

use clap::{Args, Subcommand};
use serde_json::json;

use crate::config::Config;
use crate::error::CliResult;
use crate::output::{print_json, table};

#[derive(Args)]
pub struct ContextArgs {
    #[command(subcommand)]
    pub command: Option<ContextCommand>,
}

#[derive(Subcommand)]
pub enum ContextCommand {
    /// Switch the current context.
    Use { name: String },
}

pub fn run(config: &mut Config, args: ContextArgs, json_out: bool, wide: bool) -> CliResult<()> {
    match args.command {
        None => {
            if json_out {
                let items: Vec<_> = config
                    .contexts
                    .iter()
                    .map(|(name, ctx)| {
                        json!({
                            "name": name,
                            "url": ctx.url,
                            "current": config.current.as_deref() == Some(name.as_str()),
                        })
                    })
                    .collect();
                return print_json(&items);
            }
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
                wide,
            );
            Ok(())
        }
        Some(ContextCommand::Use { name }) => {
            config.select(&name)?;
            config.save()?;
            if json_out {
                return print_json(&json!({ "current": name }));
            }
            println!("Now using {name}");
            Ok(())
        }
    }
}
