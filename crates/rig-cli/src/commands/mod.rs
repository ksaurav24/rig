//! Top-level CLI subcommand definitions and dispatch.

pub mod apps;
pub mod auth;
pub mod completions;
pub mod context;
pub mod databases;
pub mod deploy;
pub mod deployments;
pub mod env;
pub mod logs;
pub mod projects;
pub mod status;

use clap::Subcommand;

#[derive(Subcommand)]
pub enum Command {
    /// Save a server and API token as a context, and switch to it.
    Login(auth::LoginArgs),
    /// Remove a context and its stored token.
    Logout(auth::LogoutArgs),
    /// List contexts, or switch with `rig context use <name>`.
    Context(context::ContextArgs),
    /// Show who you are and how the server is doing.
    Status,
    /// List projects, or create one.
    Projects(projects::ProjectsArgs),
    /// List apps, or show details for one app.
    Apps(apps::AppsArgs),
    /// Deploy an app and follow the build log.
    Deploy(deploy::DeployArgs),
    /// List an app's deployments, or inspect/stream one by id.
    Deployments(deployments::DeploymentsArgs),
    /// Roll an app back to an earlier deployment.
    Rollback(deploy::RollbackArgs),
    /// Stop an app.
    Stop(deploy::StopArgs),
    /// Stream an app's container logs.
    Logs(logs::LogsArgs),
    /// Manage an app's environment variables.
    Env(env::EnvArgs),
    /// List databases.
    Databases,
    /// Generate shell completion scripts.
    Completions(completions::CompletionsArgs),
    /// Run a Model Context Protocol server on stdio, for AI agents.
    Mcp,
}
