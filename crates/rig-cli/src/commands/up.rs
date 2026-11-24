//! `rig up`: deploy a local project directory or pre-compiled Linux binary without Git.

use std::path::PathBuf;

use clap::Args;

use crate::{
    api::Client,
    commands::deploy::{FollowOptions, after_enqueue},
    error::CliResult,
    ops::up::{UpOptions, execute_up},
    output::{OutputOpts, parse_duration_secs},
};

#[derive(Args)]
pub struct UpArgs {
    /// Local directory or pre-compiled Linux binary to deploy (defaults to `.`).
    pub path: Option<PathBuf>,
    /// Target Rig server/VM (saved context name or server URL).
    #[arg(long, value_name = "VM")]
    pub node: Option<String>,
    /// Target app name, `project/app`, or id. Defaults to `rig.toml` or folder/binary name.
    #[arg(long)]
    pub app: Option<String>,
    /// Project name, slug, or id (used when creating a new local app on first `rig up`).
    #[arg(long)]
    pub project: Option<String>,
    /// Deploy a pre-compiled Linux ELF binary directly with zero Dockerfile required.
    #[arg(long, value_name = "PATH")]
    pub binary: Option<PathBuf>,
    /// Stream a minimal zstd delta tarball and build on the server instead of building locally.
    #[arg(long)]
    pub remote_build: bool,
    /// Container port the app listens on.
    #[arg(long)]
    pub port: Option<u16>,
    /// Dockerfile or Containerfile path relative to the directory root.
    #[arg(long)]
    pub dockerfile: Option<String>,
    /// HTTP health check path (e.g. `/health`).
    #[arg(long)]
    pub health_path: Option<String>,
    /// Optional base image override for direct binary deploys (defaults to scratch/distroless).
    #[arg(long)]
    pub base_image: Option<String>,
    /// Optional command-line arguments passed to the binary in direct binary mode (repeatable).
    #[arg(long = "arg", value_name = "ARG")]
    pub binary_args: Vec<String>,
    /// Return as soon as the deployment is queued.
    #[arg(long)]
    pub no_follow: bool,
    /// Confirm the app reports healthy (`running`) after deployment completes.
    #[arg(long)]
    pub wait_healthy: bool,
    /// Abort waiting after this duration (e.g. `10m`, `300s`).
    #[arg(long)]
    pub timeout: Option<String>,
}

pub async fn run(api: &Client, args: UpArgs, out: OutputOpts) -> CliResult<()> {
    let timeout = args.timeout.as_deref().map(parse_duration_secs).transpose()?;
    let structured = out.is_structured() || out.is_name();

    let summary = execute_up(
        api,
        &UpOptions {
            path: args.path,
            binary: args.binary,
            app: args.app,
            project: args.project,
            remote_build: args.remote_build,
            port: args.port,
            dockerfile: args.dockerfile,
            health_path: args.health_path,
            base_image: args.base_image,
            binary_args: args.binary_args,
        },
        |msg| {
            if !structured {
                eprintln!("  {msg}");
            }
        },
    )
    .await?;

    after_enqueue(
        api,
        &summary.app,
        summary.deployment,
        FollowOptions {
            no_follow: args.no_follow,
            wait_healthy: args.wait_healthy,
            timeout,
            verb: "Deploying",
        },
        out,
    )
    .await
}
