//! `rig deployments [ls|show|logs|cancel]`.

use std::time::Duration;

use clap::{Args, Subcommand};

use crate::api::{Client, DeploymentListQuery};
use crate::error::CliResult;
use crate::ops;
use crate::output::{
    OutputOpts, label, print_log, print_names, print_structured, relative_time, short,
    style_status, table,
};

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct DeploymentsArgs {
    #[command(subcommand)]
    pub command: Option<DeploymentsCommand>,
    /// App name, `project/app`, or id (shorthand for `rig deployments ls <app>`). Defaults to `rig.toml`.
    pub app: Option<String>,
    /// Maximum number of deployments to show.
    #[arg(long)]
    pub limit: Option<u32>,
    /// Show deployments created before this deployment id.
    #[arg(long)]
    pub before: Option<String>,
}

#[derive(Subcommand)]
pub enum DeploymentsCommand {
    /// List an app's deployments.
    Ls {
        /// App name, `project/app`, or id. Defaults to `rig.toml`.
        app: Option<String>,
        /// Maximum number of deployments to show (1–200).
        #[arg(long)]
        limit: Option<u32>,
        /// Cursor: return deployments created before this deployment id.
        #[arg(long)]
        before: Option<String>,
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
    /// Cancel a queued or in-progress deployment.
    Cancel {
        /// Deployment id.
        id: String,
    },
}

pub async fn run(api: &Client, args: DeploymentsArgs, out: OutputOpts) -> CliResult<()> {
    match args.command {
        Some(DeploymentsCommand::Ls { app, limit, before }) => {
            list(api, app.as_deref(), DeploymentListQuery { limit, before }, out).await
        }
        Some(DeploymentsCommand::Show { id }) => show(api, &id, out).await,
        Some(DeploymentsCommand::Logs { id, no_follow }) => logs(api, &id, no_follow, out).await,
        Some(DeploymentsCommand::Cancel { id }) => cancel(api, &id, out).await,
        None => {
            list(
                api,
                args.app.as_deref(),
                DeploymentListQuery {
                    limit: args.limit,
                    before: args.before,
                },
                out,
            )
            .await
        }
    }
}

async fn list(
    api: &Client,
    wanted_app: Option<&str>,
    q: DeploymentListQuery,
    out: OutputOpts,
) -> CliResult<()> {
    let app = ops::apps::resolve_app_or_linked(api, wanted_app).await?;
    let deps = api.list_deployments_page(&app.id, &q).await?;
    if out.is_structured() {
        return print_structured(&deps, out.format);
    }
    if out.is_name() {
        let names: Vec<String> = deps.iter().map(|d| d.id.clone()).collect();
        print_names(&names);
        return Ok(());
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
                style_status(&label(&d.status)),
                d.trigger.clone(),
                what,
                relative_time(&d.created_at),
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
        out.wide,
    );
    Ok(())
}

async fn show(api: &Client, id: &str, out: OutputOpts) -> CliResult<()> {
    let dep = api.get_deployment(id).await?;
    if out.is_structured() {
        return print_structured(&dep, out.format);
    }
    if out.is_name() {
        println!("{}", dep.id);
        return Ok(());
    }
    println!("Deployment  {}", dep.id);
    println!("App ID      {}", dep.app_id);
    println!("Status      {}", style_status(&label(&dep.status)));
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
    println!("Created     {} ({})", dep.created_at, relative_time(&dep.created_at));
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

async fn cancel(api: &Client, id: &str, out: OutputOpts) -> CliResult<()> {
    let dep = api.cancel_deployment(id).await?;
    if out.is_structured() {
        return print_structured(&dep, out.format);
    }
    if out.is_name() {
        println!("{}", dep.id);
        return Ok(());
    }
    println!("Cancelled deployment {}", short(&dep.id));
    Ok(())
}

async fn logs(api: &Client, id: &str, no_follow: bool, out: OutputOpts) -> CliResult<()> {
    let json_out = out.is_json();
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
