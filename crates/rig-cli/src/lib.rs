//! `rig`: operate a Rig server from the terminal.
//!
//! The CLI is a thin client over the same HTTP API the dashboard uses. Both
//! interactive subcommands and `rig mcp` share [`api`] and [`ops`].

pub mod api;
pub mod commands;
pub mod config;
pub mod error;
pub mod link;
pub mod mcp;
pub mod ops;
pub mod output;

use clap::Parser;

use api::Client;
use commands::Command;
use config::{Config, ResolveOptions};
use error::CliResult;
use mcp::ClientSource;
use output::{OutputFormat, OutputOpts};

#[derive(Parser)]
#[command(
    name = "rig",
    version,
    about = "Operate a Rig server from the terminal"
)]
pub struct Cli {
    /// Use this saved context instead of the current one.
    #[arg(long, global = true, env = "RIG_CONTEXT")]
    pub context: Option<String>,
    /// Target this server URL directly (pair with RIG_TOKEN for stateless CI use).
    #[arg(long, global = true, env = "RIG_URL")]
    pub server: Option<String>,
    /// Allow plain http:// to a server that is not on this machine.
    #[arg(long, global = true, env = "RIG_INSECURE")]
    pub insecure: bool,
    /// Print JSON instead of tables (shorthand for `-o json`).
    #[arg(long, global = true)]
    pub json: bool,
    /// Output format: table, json, yaml, or name.
    #[arg(short = 'o', long = "output", global = true, value_enum)]
    pub output: Option<OutputFormat>,
    /// Do not truncate table columns to terminal width.
    #[arg(long, global = true)]
    pub wide: bool,
    /// Skip interactive confirmation prompts for destructive operations.
    #[arg(short = 'y', long = "yes", global = true)]
    pub yes: bool,
    /// Log HTTP requests and timing to stderr.
    #[arg(short = 'v', long = "verbose", alias = "debug", global = true)]
    pub verbose: bool,
    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    pub fn output_opts(&self) -> OutputOpts {
        let format = if self.json {
            OutputFormat::Json
        } else {
            self.output.unwrap_or(OutputFormat::Table)
        };
        OutputOpts {
            format,
            wide: self.wide,
            yes: self.yes,
        }
    }
}

pub async fn run(cli: Cli) -> CliResult<()> {
    let out = cli.output_opts();
    let verbose = cli.verbose;

    match cli.command {
        Command::Completions(args) => {
            commands::completions::run(args);
            Ok(())
        }
        Command::Mcp => {
            let token = std::env::var("RIG_TOKEN").ok();
            mcp::serve(ClientSource {
                context: cli.context,
                server: cli.server,
                token,
                insecure: cli.insecure,
            })
            .await
        }
        Command::Login(args) => {
            let mut config = Config::load()?;
            commands::auth::run_login(&mut config, args, cli.insecure, verbose, out).await
        }
        Command::Logout(args) => {
            let mut config = Config::load()?;
            commands::auth::run_logout(&mut config, args, cli.context.as_deref(), verbose, out).await
        }
        Command::Context(args) => {
            let mut config = Config::load()?;
            commands::context::run(&mut config, args, out)
        }
        command => {
            let config = Config::load()?;
            let env_token = std::env::var("RIG_TOKEN").ok();
            let node_override = match &command {
                Command::Up(a) => a.node.as_deref(),
                Command::Adopt(a) => a.node.as_deref(),
                _ => None,
            };
            let (resolved_context, resolved_server) =
                resolve_node_override(&config, node_override, cli.context.as_deref(), cli.server.as_deref());
            let ctx = config.resolve_with(&ResolveOptions {
                context: resolved_context,
                server: resolved_server,
                token: env_token.as_deref(),
                insecure: cli.insecure,
            })?;
            let api = Client::with_options(ctx, verbose, !out.is_structured())?;
            match command {
                Command::Status => commands::status::run(&api, out).await,
                Command::Projects(args) => commands::projects::run(&api, args, out).await,
                Command::Apps(args) => commands::apps::run(&api, args, out).await,
                Command::Deploy(args) => commands::deploy::run_deploy(&api, args, out).await,
                Command::Up(args) => commands::up::run(&api, args, out).await,
                Command::Adopt(args) => commands::adopt::run(&api, args, out).await,
                Command::Start(args) => commands::deploy::run_start(&api, args, out).await,
                Command::Restart(args) => commands::deploy::run_restart(&api, args, out).await,
                Command::Rollback(args) => commands::deploy::run_rollback(&api, args, out).await,
                Command::Stop(args) => commands::deploy::run_stop(&api, args, out).await,
                Command::Deployments(args) => commands::deployments::run(&api, args, out).await,
                Command::Logs(args) => commands::logs::run(&api, args, out).await,
                Command::Exec(args) => commands::exec::run(&api, args).await,
                Command::Env(args) => commands::env::run(&api, args, out).await,
                Command::Domains(args) => commands::domains::run(&api, args, out).await,
                Command::Db(args) => commands::databases::run(&api, args, out).await,
                Command::Backup(args) => commands::backup::run(&api, args, out).await,
                Command::Metrics(args) => commands::metrics::run_metrics(&api, args, out).await,
                Command::Top(args) => commands::metrics::run_top(&api, args, out).await,
                Command::Tokens(args) => commands::tokens::run(&api, args, out).await,
                Command::Members(args) => commands::members::run(&api, args, out).await,
                Command::Audit(args) => commands::audit::run(&api, args, out).await,
                Command::Alerts(args) => commands::alerts::run(&api, args, out).await,
                Command::Webhook(args) => commands::webhook::run(&api, args, out).await,
                Command::Github(args) => commands::github::run(&api, args, out).await,
                Command::Link(args) => {
                    let active_context = cli.context.or(config.current);
                    commands::link::run(&api, args, active_context, out).await
                }
                Command::Open(args) => commands::open::run(&api, args, out).await,
                Command::Login(_)
                | Command::Logout(_)
                | Command::Context(_)
                | Command::Completions(_)
                | Command::Mcp => unreachable!(),
            }
        }
    }
}

/// Resolves `--node <vm>` as either a saved context name or a direct server URL.
pub fn resolve_node_override<'a>(
    config: &Config,
    node: Option<&'a str>,
    cli_context: Option<&'a str>,
    cli_server: Option<&'a str>,
) -> (Option<&'a str>, Option<&'a str>) {
    let Some(n) = node.map(str::trim).filter(|s| !s.is_empty()) else {
        return (cli_context, cli_server);
    };
    if config.contexts.contains_key(n) {
        (Some(n), None)
    } else if n.contains("://") || n.contains('.') || n.contains(':') {
        (None, Some(n))
    } else {
        // Treat unknown bare names as context names so Config::resolve reports a crisp context error.
        (Some(n), None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_parses_without_conflicts() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }
}
