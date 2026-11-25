//! `rig apps [ls|show|create|set|rm]`.

use clap::{Args, Subcommand};
use rig_api::{CreateAppRequest, SetEnvVarRequest, SourceKind, UpdateAppRequest};
use serde_json::json;

use crate::api::Client;
use crate::error::{CliError, CliResult};
use crate::ops;
use crate::output::{
    OutputOpts, confirm, domain_url, label, print_names, print_structured, short, style_status,
    table,
};

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
        /// App name, `project/app`, or id. Defaults to `rig.toml`.
        app: Option<String>,
    },
    /// Create a new app from a prebuilt container image or a Git repository.
    Create {
        /// App name.
        name: String,
        /// Project name, slug, or id.
        #[arg(long)]
        project: String,
        /// Prebuilt container image (e.g. `nginx:alpine`). Mutually exclusive with `--git`.
        #[arg(long, conflicts_with = "git")]
        image: Option<String>,
        /// Git repository URL (`https://` or `ssh://`). Mutually exclusive with `--image`.
        #[arg(long, conflicts_with = "image")]
        git: Option<String>,
        /// Git branch to deploy (defaults to repo default branch).
        #[arg(long)]
        branch: Option<String>,
        /// Dockerfile path relative to repo root.
        #[arg(long)]
        dockerfile: Option<String>,
        /// Container port the app listens on.
        #[arg(long, default_value_t = 80)]
        port: u16,
        /// Memory limit in MB.
        #[arg(long)]
        memory: Option<u32>,
        /// CPU limit in millicores (e.g. 500 = 0.5 CPU).
        #[arg(long)]
        cpu: Option<u32>,
        /// HTTP health check path (e.g. `/health`).
        #[arg(long)]
        health: Option<String>,
        /// Persistent volume mount paths inside the container (repeatable).
        #[arg(long = "volume")]
        volumes: Vec<String>,
        /// Initial environment variables (`KEY=VALUE`, repeatable; stored as secrets).
        #[arg(long = "env", value_name = "KEY=VALUE")]
        env: Vec<String>,
    },
    /// Update an app's settings (takes effect on the next deploy).
    Set {
        /// App name, `project/app`, or id. Defaults to `rig.toml`.
        app: Option<String>,
        /// Rename the app.
        #[arg(long)]
        name: Option<String>,
        /// Change container image (image apps).
        #[arg(long)]
        image: Option<String>,
        /// Change Git repository URL (git apps).
        #[arg(long)]
        git: Option<String>,
        /// Change Git branch (empty string resets to repo default).
        #[arg(long)]
        branch: Option<String>,
        /// Change Dockerfile path.
        #[arg(long)]
        dockerfile: Option<String>,
        /// Change container port.
        #[arg(long)]
        port: Option<u16>,
        /// Change memory limit in MB (0 removes limit).
        #[arg(long)]
        memory: Option<u32>,
        /// Change CPU limit in millicores (0 removes limit).
        #[arg(long)]
        cpu: Option<u32>,
        /// Change health check path (empty string switches to TCP check).
        #[arg(long)]
        health: Option<String>,
        /// Enable or disable deploy-on-push for Git apps.
        #[arg(long)]
        auto_deploy: Option<bool>,
    },
    /// Delete an app, its containers, volumes, variables, and domains.
    Rm {
        /// App name, `project/app`, or id. Defaults to `rig.toml`.
        app: Option<String>,
    },
}

pub async fn run(api: &Client, args: AppsArgs, out: OutputOpts) -> CliResult<()> {
    match args.command {
        None => list(api, args.project.as_deref(), out).await,
        Some(AppsCommand::Ls { project }) => {
            list(api, project.as_deref().or(args.project.as_deref()), out).await
        }
        Some(AppsCommand::Show { app }) => show(api, app.as_deref(), out).await,
        Some(AppsCommand::Create {
            name,
            project,
            image,
            git,
            branch,
            dockerfile,
            port,
            memory,
            cpu,
            health,
            volumes,
            env,
        }) => {
            let source_kind = match (&image, &git) {
                (Some(_), None) => SourceKind::Image,
                (None, Some(_)) => SourceKind::Git,
                _ => {
                    return Err(CliError::message(
                        "specify either --image <ref> or --git <url>",
                    ));
                }
            };
            let env_pairs = ops::env::parse_pairs(&env)?;
            let proj = ops::projects::resolve_project(api, &project).await?;
            let req = CreateAppRequest {
                project_id: proj.id,
                name,
                source_kind,
                image,
                git_url: git,
                git_branch: branch,
                dockerfile_path: dockerfile,
                port,
                memory_mb: memory,
                cpu_millis: cpu,
                health_path: health,
                volumes,
                env: env_pairs
                    .into_iter()
                    .map(|(key, value)| SetEnvVarRequest {
                        key,
                        value,
                        is_secret: true,
                    })
                    .collect(),
            };
            let created = api.create_app(&req).await?;
            if out.is_structured() {
                return print_structured(&created, out.format);
            }
            if out.is_name() {
                println!("{}/{}", created.project_name, created.name);
                return Ok(());
            }
            println!(
                "Created app {}/{} ({})",
                created.project_name, created.name, created.id
            );
            println!("Deploy it with: rig deploy {}/{}", created.project_name, created.name);
            Ok(())
        }
        Some(AppsCommand::Set {
            app,
            name,
            image,
            git,
            branch,
            dockerfile,
            port,
            memory,
            cpu,
            health,
            auto_deploy,
        }) => {
            let existing = ops::apps::resolve_app_or_linked(api, app.as_deref()).await?;
            let req = UpdateAppRequest {
                name,
                image,
                git_url: git,
                git_branch: branch,
                dockerfile_path: dockerfile,
                port,
                memory_mb: memory,
                cpu_millis: cpu,
                health_path: health,
                auto_deploy,
                volumes: None,
            };
            let updated = api.update_app(&existing.id, &req).await?;
            if out.is_structured() {
                return print_structured(&updated, out.format);
            }
            if out.is_name() {
                println!("{}/{}", updated.project_name, updated.name);
                return Ok(());
            }
            println!("Updated {}/{}", updated.project_name, updated.name);
            println!("Changes apply on the next deploy: rig deploy {}", updated.name);
            Ok(())
        }
        Some(AppsCommand::Rm { app }) => {
            let existing = ops::apps::resolve_app_or_linked(api, app.as_deref()).await?;
            confirm(
                &format!(
                    "Delete app {}/{} and all of its containers, volumes, and domains?",
                    existing.project_name, existing.name
                ),
                out.yes,
            )?;
            api.delete_app(&existing.id).await?;
            if out.is_structured() {
                return print_structured(
                    &json!({
                        "deleted": format!("{}/{}", existing.project_name, existing.name),
                        "id": existing.id,
                    }),
                    out.format,
                );
            }
            if out.is_name() {
                println!("{}/{}", existing.project_name, existing.name);
                return Ok(());
            }
            println!("Deleted {}/{}", existing.project_name, existing.name);
            Ok(())
        }
    }
}

async fn list(api: &Client, project: Option<&str>, out: OutputOpts) -> CliResult<()> {
    let apps = ops::apps::list_apps(api, project).await?;
    if out.is_structured() {
        return print_structured(&apps, out.format);
    }
    if out.is_name() {
        let names: Vec<String> = apps
            .iter()
            .map(|a| format!("{}/{}", a.project_name, a.name))
            .collect();
        print_names(&names);
        return Ok(());
    }
    let rows = apps
        .iter()
        .map(|a| {
            let source = a
                .image
                .clone()
                .or_else(|| a.git_url.clone())
                .unwrap_or_default();
            let url = a
                .domains
                .first()
                .map(|d| domain_url(d, false))
                .unwrap_or_default();
            vec![
                format!("{}/{}", a.project_name, a.name),
                style_status(&label(&a.status)),
                source,
                url,
            ]
        })
        .collect();
    table(
        &["APP", "STATUS", "SOURCE", "URL"],
        rows,
        Some("No apps found."),
        out.wide,
    );
    Ok(())
}

async fn show(api: &Client, wanted: Option<&str>, out: OutputOpts) -> CliResult<()> {
    let app = ops::apps::resolve_app_or_linked(api, wanted).await?;
    let app = api.get_app(&app.id).await?;
    if out.is_structured() {
        return print_structured(&app, out.format);
    }
    if out.is_name() {
        println!("{}/{}", app.project_name, app.name);
        return Ok(());
    }
    println!("{}/{} ({})", app.project_name, app.name, app.id);
    println!("Status      {}", style_status(&label(&app.status)));
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
        println!("Loopback    http://127.0.0.1:{hp}");
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
        let urls: Vec<String> = app.domains.iter().map(|d| domain_url(d, false)).collect();
        println!("Domains     {}", urls.join(", "));
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
