//! Top-level CLI subcommand definitions and dispatch.

pub mod adopt;
pub mod alerts;
pub mod apps;
pub mod audit;
pub mod auth;
pub mod backup;
pub mod completions;
pub mod context;
pub mod databases;
pub mod deploy;
pub mod deployments;
pub mod devices;
pub mod domains;
pub mod env;
pub mod exec;
pub mod github;
pub mod link;
pub mod logs;
pub mod members;
pub mod metrics;
pub mod open;
pub mod projects;
pub mod setup;
pub mod status;
pub mod tokens;
pub mod up;
pub mod webhook;

use clap::Subcommand;

#[derive(Subcommand)]
pub enum Command {
    /// Initialize a freshly installed (or headless) Rig server without a browser.
    Setup(setup::SetupArgs),
    /// Save a server and API token as a context, and switch to it.
    Login(auth::LoginArgs),
    /// Remove a context and its stored token (optionally revoking it on the server).
    Logout(auth::LogoutArgs),
    /// Manage saved server contexts (`use`, `current`, `rename`).
    Context(context::ContextArgs),
    /// Link the current directory to a Rig app via `rig.toml`.
    Link(link::LinkArgs),
    /// Open an app's URL (or the Rig dashboard) in your browser.
    Open(open::OpenArgs),
    /// Show who you are and how the server is doing.
    Status,
    /// Manage projects (`ls`, `create`, `rename`, `rm`).
    Projects(projects::ProjectsArgs),
    /// Manage apps (`ls`, `show`, `create`, `set`, `rm`).
    Apps(apps::AppsArgs),
    /// Deploy an app and follow the build log.
    Deploy(deploy::DeployArgs),
    /// Deploy a local directory or pre-compiled Linux binary directly without Git.
    Up(up::UpArgs),
    /// Discover and adopt running Docker/Podman containers into Rig with zero downtime.
    Adopt(adopt::AdoptArgs),
    /// Start a stopped app.
    Start(deploy::StartArgs),
    /// Restart an app with zero-downtime swap.
    Restart(deploy::RestartArgs),
    /// Roll an app back to an earlier deployment.
    Rollback(deploy::RollbackArgs),
    /// Stop an app.
    Stop(deploy::StopArgs),
    /// Inspect, stream, or cancel deployments.
    Deployments(deployments::DeploymentsArgs),
    /// Stream or query an app's container logs.
    Logs(logs::LogsArgs),
    /// Open an interactive shell inside a running app container.
    Exec(exec::ExecArgs),
    /// Manage an app's environment variables (`ls`, `set`, `get`, `rm`, `import`, `pull`, `diff`).
    Env(env::EnvArgs),
    /// Manage an app's custom domains (`ls`, `add`, `rm`).
    Domains(domains::DomainsArgs),
    /// Manage databases (`ls`, `engines`, `create`, `show`, `start`, `stop`, `rm`, `url`, `connect`).
    #[command(alias = "databases")]
    Db(databases::DbArgs),
    /// Manage database backups (`ls`, `create`, `download`, `restore`, `rm`).
    #[command(alias = "backups")]
    Backup(backup::BackupArgs),
    /// Show CPU, memory, and network metrics for the server, an app, or a database.
    Metrics(metrics::MetricsArgs),
    /// Live overview of server load, running apps, and databases.
    Top(metrics::TopArgs),
    /// Manage your API tokens (`ls`, `create`, `revoke`).
    Tokens(tokens::TokensArgs),
    /// Pair and manage Rig Desktop / mobile devices (`pair`, `ls`, `revoke`).
    Devices(devices::DevicesArgs),
    /// Manage organization members and roles (`ls`, `add`, `role`, `rm`, `reset-password`).
    Members(members::MembersArgs),
    /// View the organization's audit log.
    Audit(audit::AuditArgs),
    /// Manage alert channels (`ls`, `add`, `set`, `rm`, `test`).
    Alerts(alerts::AlertsArgs),
    /// Inspect or rotate an app's push-to-deploy webhook.
    Webhook(webhook::WebhookArgs),
    /// Inspect the organization's GitHub App and repositories.
    Github(github::GithubArgs),
    /// Generate shell completion scripts.
    Completions(completions::CompletionsArgs),
    /// Run a Model Context Protocol server on stdio, for AI agents.
    Mcp,
}
