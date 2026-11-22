//! `rig apps` and `rig apps show <app>`.

use clap::{Args, Subcommand};

use crate::api::Client;
use crate::error::CliResult;
use crate::ops;
use crate::output::{label, print_json, short, table};

#[derive(Args)]
pub struct AppsArgs {
    /// Only apps in this project (name or id).
    #[arg(long)]
    pub project: Option<String>,
    #[command(subcommand)]
    pub command: Option<AppsCommand>,
}

#[derive(Subcommand)]
pub enum AppsCommand {
    /// List apps.
    Ls {
        /// Only apps in this project (name or id).
        #[arg(long)]
        project: Option<String>,
    },
    /// Show full configuration, status, and domains for one app.
    Show {
        /// App name, `project/app`, or id.
        app: String,
    },
}

pub async fn run(api: &Client, args: AppsArgs, json_out: bool, wide: bool) -> CliResult<()> {
    match args.command {
        None => list(api, args.project.as_deref(), json_out, wide).await,
        Some(AppsCommand::Ls { project }) => {
            list(api, project.as_deref().or(args.project.as_deref()), json_out, wide).await
        }
        Some(AppsCommand::Show { app }) => show(api, &app, json_out).await,
    }
}

async fn list(api: &Client, project: Option<&str>, json_out: bool, wide: bool) -> CliResult<()> {
    let apps = ops::apps::list_apps(api, project).await?;
    if json_out {
        return print_json(&apps);
    }
    let rows = apps
        .iter()
        .map(|a| {
            let source = a
                .image
                .clone()
                .or_else(|| a.git_url.clone())
                .unwrap_or_default();
            let url = a.domains.first().cloned().unwrap_or_default();
            vec![
                format!("{}/{}", a.project_name, a.name),
                label(&a.status),
                source,
                url,
            ]
        })
        .collect();
    table(
        &["APP", "STATUS", "SOURCE", "DOMAIN"],
        rows,
        Some("No apps found."),
        wide,
    );
    Ok(())
}

async fn show(api: &Client, wanted: &str, json_out: bool) -> CliResult<()> {
    let app = ops::apps::resolve_app(api, wanted).await?;
    // Fetch fresh single-app view so details are guaranteed current.
    let app = api.get_app(&app.id).await?;
    if json_out {
        return print_json(&app);
    }
    println!("{}/{} ({})", app.project_name, app.name, app.id);
    println!("Status      {}", label(&app.status));
    println!("Source      {}", label(&app.source_kind));
    if let Some(image) = &app.image {
        println!("Image       {image}");
    }
    if let Some(git_url) = &app.git_url {
        let branch = app.git_branch.as_deref().unwrap_or("(default)");
        println!("Git         {git_url} ({branch})");
        println!("Dockerfile  {}", app.dockerfile_path);
        println!(
            "Auto-deploy {}",
            if app.auto_deploy { "on" } else { "off" }
        );
    }
    println!("Port        {}", app.port);
    if let Some(hp) = app.host_port {
        println!("Loopback    127.0.0.1:{hp}");
    }
    if let Some(mem) = app.memory_mb {
        println!("Memory      {mem} MB");
    }
    if let Some(cpu) = app.cpu_millis {
        println!("CPU         {cpu}m");
    }
    if let Some(health) = &app.health_path {
        println!("Health      {health}");
    }
    if let Some(dep_id) = &app.current_deployment_id {
        println!("Deployment  {} ({dep_id})", short(dep_id));
    }
    if !app.domains.is_empty() {
        println!("Domains     {}", app.domains.join(", "));
    }
    if !app.volumes.is_empty() {
        println!("Volumes     {}", app.volumes.join(", "));
    }
    if let Some(crash) = &app.last_crash {
        println!(
            "Last crash  {} ({} restarts): {}",
            crash.at, crash.restarts, crash.reason
        );
    }
    Ok(())
}
