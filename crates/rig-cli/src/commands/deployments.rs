//! `rig deployments <app>`, `rig deployments ls <app>`, `rig deployments show <id>`,
//! and `rig deployments logs <id>`.

use std::time::Duration;

use clap::{Args, Subcommand};

use crate::api::Client;
use crate::error::{CliError, CliResult};
use crate::ops;
use crate::output::{label, print_json, print_log, short, table};

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct DeploymentsArgs {
    #[command(subcommand)]
    pub command: Option<DeploymentsCommand>,
    /// App name, `project/app`, or id (shorthand for `rig deployments ls <app>`).
    pub app: Option<String>,
}

#[derive(Subcommand)]
pub enum DeploymentsCommand {
    /// List an app's deployments.
    Ls {
        /// App name, `project/app`, or id.
        app: String,
    },
    /// Show details of one deployment by id.
    Show {
        /// Deployment id.
        id: String,
    },
    /// Stream or print build and deploy logs for a deployment by id.
    Logs {
        /// Deployment id.
        id: String,
        /// Print currently available logs and exit without waiting if still in progress.
        #[arg(long)]
        no_follow: bool,
    },
}

pub async fn run(
    api: &Client,
    args: DeploymentsArgs,
    json_out: bool,
    wide: bool,
) -> CliResult<()> {
    match args.command {
        Some(DeploymentsCommand::Ls { app }) => list(api, &app, json_out, wide).await,
        Some(DeploymentsCommand::Show { id }) => show(api, &id, json_out).await,
        Some(DeploymentsCommand::Logs { id, no_follow }) => {
            logs(api, &id, no_follow, json_out).await
        }
        None => match args.app {
            Some(app) => list(api, &app, json_out, wide).await,
            None => Err(CliError::message(
                "specify an app or subcommand: rig deployments <app> | show <id> | logs <id>",
            )),
        },
    }
}

async fn list(api: &Client, wanted_app: &str, json_out: bool, wide: bool) -> CliResult<()> {
    let app = ops::apps::resolve_app(api, wanted_app).await?;
    let deps = api.list_deployments(&app.id).await?;
    if json_out {
        return print_json(&deps);
    }
    let rows = deps
        .iter()
        .map(|d| {
            let what = d
                .commit_sha
                .as_ref()
                .map(|sha| sha.chars().take(8).collect())
                .or_else(|| d.image_ref.clone())
                .unwrap_or_default();
            vec![
                short(&d.id),
                label(&d.status),
                d.trigger.clone(),
                what,
                d.created_at.clone(),
            ]
        })
        .collect();
    table(
        &["ID", "STATUS", "TRIGGER", "VERSION", "CREATED"],
        rows,
        Some(&format!(
            "No deployments for {} yet. Deploy it with: rig deploy {}",
            app.name, app.name
        )),
        wide,
    );
    Ok(())
}

async fn show(api: &Client, id: &str, json_out: bool) -> CliResult<()> {
    let dep = api.get_deployment(id).await?;
    if json_out {
        return print_json(&dep);
    }
    println!("Deployment  {}", dep.id);
    println!("App ID      {}", dep.app_id);
    println!("Status      {}", label(&dep.status));
    println!("Trigger     {}", dep.trigger);
    if let Some(image) = &dep.image_ref {
        println!("Image       {image}");
    }
    if let Some(sha) = &dep.commit_sha {
        println!("Commit      {sha}");
    }
    if let Some(msg) = &dep.commit_msg {
        println!("Message     {msg}");
    }
    println!("Created     {}", dep.created_at);
    if let Some(started) = &dep.started_at {
        println!("Started     {started}");
    }
    if let Some(finished) = &dep.finished_at {
        println!("Finished    {finished}");
    }
    if let Some(err) = &dep.error {
        println!("Error       {err}");
    }
    Ok(())
}

async fn logs(api: &Client, id: &str, no_follow: bool, json_out: bool) -> CliResult<()> {
    if no_follow {
        let dep = api.get_deployment(id).await?;
        let stream = ops::deployments::follow_deployment(api, &dep.id, |data| {
            print_log(data, json_out);
        });
        if dep.status.is_finished() {
            stream.await?;
        } else if let Ok(res) = tokio::time::timeout(Duration::from_millis(1500), stream).await {
            res?;
        }
        return Ok(());
    }

    let follow_fut = ops::deployments::follow_deployment(api, id, |data| {
        print_log(data, json_out);
    });

    tokio::select! {
        res = follow_fut => {
            res?;
            Ok(())
        }
        _ = tokio::signal::ctrl_c() => Ok(()),
    }
}
