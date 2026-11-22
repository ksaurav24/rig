//! `rig`: operate a Rig server from the terminal.
//!
//! The CLI is a thin client over the same HTTP API the dashboard uses. Both
//! interactive subcommands and `rig mcp` share [`api`] and [`ops`].

pub mod api;
pub mod commands;
pub mod config;
pub mod error;
pub mod mcp;
pub mod ops;
pub mod output;

use clap::Parser;

use api::Client;
use commands::Command;
use config::{Config, ResolveOptions};
use error::CliResult;
use mcp::ClientSource;

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
    /// Print JSON instead of tables.
    #[arg(long, global = true)]
    pub json: bool,
    /// Do not truncate table columns to terminal width.
    #[arg(long, global = true)]
    pub wide: bool,
    #[command(subcommand)]
    pub command: Command,
}

pub async fn run(cli: Cli) -> CliResult<()> {
    let json = cli.json;
    let wide = cli.wide;

    // Commands that do not require an existing signed-in context.
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
            commands::auth::run_login(&mut config, args, cli.insecure, json).await
        }
        Command::Logout(args) => {
            let mut config = Config::load()?;
            commands::auth::run_logout(&mut config, args, cli.context.as_deref(), json)
        }
        Command::Context(args) => {
            let mut config = Config::load()?;
            commands::context::run(&mut config, args, json, wide)
        }
        command => {
            let config = Config::load()?;
            let env_token = std::env::var("RIG_TOKEN").ok();
            let ctx = config.resolve_with(&ResolveOptions {
                context: cli.context.as_deref(),
                server: cli.server.as_deref(),
                token: env_token.as_deref(),
                insecure: cli.insecure,
            })?;
            let api = Client::new(ctx)?;
            match command {
                Command::Status => commands::status::run(&api, json).await,
                Command::Projects(args) => commands::projects::run(&api, args, json, wide).await,
                Command::Apps(args) => commands::apps::run(&api, args, json, wide).await,
                Command::Deploy(args) => commands::deploy::run_deploy(&api, args, json).await,
                Command::Deployments(args) => {
                    commands::deployments::run(&api, args, json, wide).await
                }
                Command::Rollback(args) => commands::deploy::run_rollback(&api, args, json).await,
                Command::Stop(args) => commands::deploy::run_stop(&api, args, json).await,
                Command::Logs(args) => commands::logs::run(&api, args, json).await,
                Command::Env(args) => commands::env::run(&api, args, json, wide).await,
                Command::Databases => commands::databases::run(&api, json, wide).await,
                Command::Login(_)
                | Command::Logout(_)
                | Command::Context(_)
                | Command::Completions(_)
                | Command::Mcp => unreachable!(),
            }
        }
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
