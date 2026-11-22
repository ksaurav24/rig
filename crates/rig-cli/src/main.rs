//! `rig`: operate a Rig server from the terminal.
//!
//! The CLI is a thin client over the same HTTP API the dashboard uses. It
//! authenticates with an API token and keeps one "context" per server, so
//! several servers can be driven from one machine.

mod client;
mod config;
mod mcp;

use std::io::{IsTerminal, Write};

use anyhow::{Context as _, Result, bail};
use clap::{Parser, Subcommand};
use rig_api::{
    App, CreateProjectRequest, Database, Deployment, DeploymentStatus, EnvVar, LogLine, Me,
    Project, RevealedEnvVar, SetEnvVarRequest, SystemInfo,
};
use serde::Serialize;

use client::Client;
use config::{Config, Context};

#[derive(Parser)]
#[command(
    name = "rig",
    version,
    about = "Operate a Rig server from the terminal"
)]
struct Cli {
    /// Use this context instead of the current one.
    #[arg(long, global = true, env = "RIG_CONTEXT")]
    context: Option<String>,
    /// Print JSON instead of tables.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Save a server and API token as a context, and switch to it.
    Login {
        /// Server address, e.g. https://rig.example.com
        url: String,
        /// Name for this context. Defaults to the server's host name.
        #[arg(long)]
        name: Option<String>,
        /// API token. Prefer RIG_TOKEN or the prompt, which keep it out of shell history.
        #[arg(long, env = "RIG_TOKEN", hide_env_values = true)]
        token: Option<String>,
        /// Allow plain http:// to a server that is not on this machine.
        #[arg(long)]
        insecure: bool,
    },
    /// Remove a context and its stored token.
    Logout { name: Option<String> },
    /// List contexts, or switch with `rig context use <name>`.
    Context {
        #[command(subcommand)]
        command: Option<ContextCommand>,
    },
    /// Show who you are and how the server is doing.
    Status,
    /// List projects, or create one.
    Projects {
        #[command(subcommand)]
        command: Option<ProjectCommand>,
    },
    /// List apps.
    Apps {
        /// Only apps in this project (name or id).
        #[arg(long)]
        project: Option<String>,
    },
    /// Deploy an app and follow the build log.
    Deploy {
        /// App name or id. Use `project/app` when the name is not unique.
        app: String,
        /// Return as soon as the deployment is queued.
        #[arg(long)]
        no_follow: bool,
    },
    /// List an app's deployments.
    Deployments { app: String },
    /// Roll an app back to an earlier deployment.
    Rollback {
        app: String,
        /// Deployment id (or its first characters). Defaults to the previous live one.
        deployment: Option<String>,
        #[arg(long)]
        no_follow: bool,
    },
    /// Stop an app.
    Stop { app: String },
    /// Stream an app's container logs.
    Logs { app: String },
    /// Manage an app's environment variables.
    Env {
        #[command(subcommand)]
        command: EnvCommand,
    },
    /// List databases.
    Databases,
    /// Run a Model Context Protocol server on stdio, for AI agents.
    Mcp,
}

#[derive(Subcommand)]
enum ContextCommand {
    /// Switch the current context.
    Use { name: String },
}

#[derive(Subcommand)]
enum ProjectCommand {
    /// Create a project.
    Create { name: String },
}

#[derive(Subcommand)]
enum EnvCommand {
    /// List variables. Secret values are hidden.
    Ls { app: String },
    /// Set variables: `rig env set web KEY=value OTHER=value`. Stored as secrets unless --plain.
    Set {
        app: String,
        #[arg(required = true, value_name = "KEY=VALUE")]
        pairs: Vec<String>,
        /// Store as a plain, readable value instead of a secret.
        #[arg(long)]
        plain: bool,
    },
    /// Print one variable's value. Reading a secret is recorded in the audit log.
    Get { app: String, key: String },
    /// Delete a variable.
    Rm { app: String, key: String },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli).await {
        eprintln!("rig: {e:#}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<()> {
    let json = cli.json;
    let mut config = Config::load()?;

    // Commands that work without a server.
    match &cli.command {
        Command::Login {
            url,
            name,
            token,
            insecure,
        } => {
            return login(&mut config, url, name.as_deref(), token.clone(), *insecure).await;
        }
        Command::Logout { name } => {
            let name = name.clone().or(cli.context).or(config.current.clone());
            let Some(name) = name else {
                bail!("there is no context to remove")
            };
            config.remove(&name)?;
            config.save()?;
            println!("Removed {name}");
            return Ok(());
        }
        Command::Context { command: None } => {
            if config.contexts.is_empty() {
                println!("No contexts. Add one with: rig login <url>");
            }
            let rows = config
                .contexts
                .iter()
                .map(|(name, ctx)| {
                    let mark = if config.current.as_deref() == Some(name) {
                        "*"
                    } else {
                        ""
                    };
                    vec![mark.to_string(), name.clone(), ctx.url.clone()]
                })
                .collect();
            table(&["", "NAME", "SERVER"], rows);
            return Ok(());
        }
        Command::Context {
            command: Some(ContextCommand::Use { name }),
        } => {
            config.select(name)?;
            config.save()?;
            println!("Now using {name}");
            return Ok(());
        }
        _ => {}
    }

    let api = Client::new(config.resolve(cli.context.as_deref())?)?;
    match cli.command {
        Command::Login { .. } | Command::Logout { .. } | Command::Context { .. } => unreachable!(),
        Command::Mcp => return mcp::serve(api).await,
        Command::Status => {
            let me: Me = api.get("/api/v1/auth/me").await?;
            let sys: SystemInfo = api.get("/api/v1/system").await?;
            if json {
                return print_json(&serde_json::json!({ "me": me, "system": sys }));
            }
            println!(
                "{} ({}) in {}, as {}",
                me.user.name,
                me.user.email,
                me.org.name,
                label(&me.role)
            );
            println!("Server   {} · Rig {}", api.base(), sys.version);
            match (&sys.engine, &sys.engine_version) {
                (Some(engine), Some(version)) => println!("Engine   {engine} {version}"),
                _ => println!("Engine   not reachable"),
            }
            println!(
                "Proxy    {}",
                if sys.proxy_ready {
                    "ready"
                } else {
                    "not reachable"
                }
            );
            println!(
                "Apps     {} running of {}",
                sys.apps_running, sys.apps_total
            );
            println!(
                "Load     CPU {:.0}% · memory {} of {} · disk {} of {}",
                sys.cpu_percent,
                bytes(sys.memory_used),
                bytes(sys.memory_total),
                bytes(sys.disk_used),
                bytes(sys.disk_total)
            );
        }
        Command::Projects { command: None } => {
            let projects: Vec<Project> = api.get("/api/v1/projects").await?;
            if json {
                return print_json(&projects);
            }
            let rows = projects
                .iter()
                .map(|p| vec![p.name.clone(), p.app_count.to_string(), p.id.clone()])
                .collect();
            table(&["NAME", "APPS", "ID"], rows);
        }
        Command::Projects {
            command: Some(ProjectCommand::Create { name }),
        } => {
            let body = CreateProjectRequest {
                name,
                description: String::new(),
            };
            let project: Project = api.post("/api/v1/projects", Some(&body)).await?;
            if json {
                return print_json(&project);
            }
            println!("Created project {}", project.name);
        }
        Command::Apps { project } => {
            let mut apps: Vec<App> = api.get("/api/v1/apps").await?;
            if let Some(project) = project {
                apps.retain(|a| {
                    a.project_id == project || a.project_name.eq_ignore_ascii_case(&project)
                });
            }
            if json {
                return print_json(&apps);
            }
            let rows = apps
                .iter()
                .map(|a| {
                    let source = a.image.clone().or(a.git_url.clone()).unwrap_or_default();
                    let url = a.domains.first().cloned().unwrap_or_default();
                    vec![
                        format!("{}/{}", a.project_name, a.name),
                        label(&a.status),
                        source,
                        url,
                    ]
                })
                .collect();
            table(&["APP", "STATUS", "SOURCE", "DOMAIN"], rows);
        }
        Command::Deploy { app, no_follow } => {
            let app = find_app(&api, &app).await?;
            let dep: Deployment = api
                .post(&format!("/api/v1/apps/{}/deploy", app.id), None::<&()>)
                .await?;
            after_enqueue(&api, &app, dep, no_follow, json).await?;
        }
        Command::Deployments { app } => {
            let app = find_app(&api, &app).await?;
            let deps: Vec<Deployment> = api
                .get(&format!("/api/v1/apps/{}/deployments", app.id))
                .await?;
            if json {
                return print_json(&deps);
            }
            let rows = deps
                .iter()
                .map(|d| {
                    let what = d
                        .commit_sha
                        .as_ref()
                        .map(|sha| sha.chars().take(8).collect())
                        .or(d.image_ref.clone())
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
            table(&["ID", "STATUS", "TRIGGER", "VERSION", "CREATED"], rows);
        }
        Command::Rollback {
            app,
            deployment,
            no_follow,
        } => {
            let app = find_app(&api, &app).await?;
            let deps: Vec<Deployment> = api
                .get(&format!("/api/v1/apps/{}/deployments", app.id))
                .await?;
            let target = match deployment {
                Some(wanted) => deps
                    .iter()
                    .find(|d| d.id == wanted || short(&d.id) == wanted || d.id.ends_with(&wanted))
                    .with_context(|| format!("{} has no deployment {wanted}", app.name))?,
                None => deps
                    .iter()
                    .find(|d| d.status == DeploymentStatus::Superseded && d.image_ref.is_some())
                    .with_context(|| {
                        format!("{} has no earlier deployment to roll back to", app.name)
                    })?,
            };
            let dep: Deployment = api
                .post(
                    &format!("/api/v1/deployments/{}/rollback", target.id),
                    None::<&()>,
                )
                .await?;
            after_enqueue(&api, &app, dep, no_follow, json).await?;
        }
        Command::Stop { app } => {
            let app = find_app(&api, &app).await?;
            let _: App = api
                .post(&format!("/api/v1/apps/{}/stop", app.id), None::<&()>)
                .await?;
            println!("Stopped {}", app.name);
        }
        Command::Logs { app } => {
            let app = find_app(&api, &app).await?;
            if app.current_deployment_id.is_none() {
                bail!(
                    "{} is not running, so it has no logs. Deploy it first",
                    app.name
                );
            }
            api.events(&format!("/api/v1/apps/{}/logs", app.id), |event, data| {
                if event == "log" {
                    print_log(data);
                }
            })
            .await?;
        }
        Command::Env { command } => env(&api, command, json).await?,
        Command::Databases => {
            let dbs: Vec<Database> = api.get("/api/v1/databases").await?;
            if json {
                return print_json(&dbs);
            }
            let rows = dbs
                .iter()
                .map(|d| {
                    vec![
                        format!("{}/{}", d.project_name, d.name),
                        format!("{} {}", label(&d.engine), d.version),
                        label(&d.status),
                        format!("{}:{}", d.host, d.port),
                    ]
                })
                .collect();
            table(
                &["DATABASE", "ENGINE", "STATUS", "ADDRESS IN PROJECT"],
                rows,
            );
        }
    }
    Ok(())
}

async fn login(
    config: &mut Config,
    url: &str,
    name: Option<&str>,
    token: Option<String>,
    insecure: bool,
) -> Result<()> {
    let url = config::normalize_url(url, insecure)?;
    let token = match token {
        Some(t) => t,
        None => {
            if !std::io::stdin().is_terminal() {
                bail!(
                    "no token given. Set RIG_TOKEN, or run this in a terminal to be asked for it"
                );
            }
            eprint!("API token (create one in the dashboard under Settings → API tokens): ");
            std::io::stderr().flush().ok();
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            line
        }
    };
    let token = token.trim().to_string();
    if token.is_empty() {
        bail!("the token is empty");
    }
    let ctx = Context {
        url: url.to_string(),
        token,
    };
    // Prove the pair works before saving anything.
    let me: Me = Client::new(ctx.clone())?
        .get("/api/v1/auth/me")
        .await
        .context("could not sign in with that token")?;
    let name = match name {
        Some(n) => n.to_string(),
        None => url.host_str().unwrap_or("default").to_string(),
    };
    config.contexts.insert(name.clone(), ctx);
    config.current = Some(name.clone());
    config.save()?;
    println!(
        "Signed in to {} as {} ({}). Context: {name}",
        me.org.name,
        me.user.email,
        label(&me.role)
    );
    Ok(())
}

async fn env(api: &Client, command: EnvCommand, json: bool) -> Result<()> {
    match command {
        EnvCommand::Ls { app } => {
            let app = find_app(api, &app).await?;
            let vars: Vec<EnvVar> = api.get(&format!("/api/v1/apps/{}/env", app.id)).await?;
            if json {
                return print_json(&vars);
            }
            let rows = vars
                .iter()
                .map(|v| {
                    vec![
                        v.key.clone(),
                        v.value.clone().unwrap_or_else(|| "(secret)".into()),
                    ]
                })
                .collect();
            table(&["KEY", "VALUE"], rows);
        }
        EnvCommand::Set { app, pairs, plain } => {
            let app = find_app(api, &app).await?;
            // Check every pair before changing anything.
            let mut parsed = Vec::with_capacity(pairs.len());
            for pair in &pairs {
                let (key, value) = pair
                    .split_once('=')
                    .with_context(|| format!("`{pair}` is not KEY=VALUE"))?;
                parsed.push((key, value));
            }
            for (key, value) in parsed {
                let body = SetEnvVarRequest {
                    key: key.into(),
                    value: value.into(),
                    is_secret: !plain,
                };
                let _: EnvVar = api
                    .put(&format!("/api/v1/apps/{}/env", app.id), &body)
                    .await?;
                println!("Set {key}");
            }
            println!("Changes apply on the next deploy: rig deploy {}", app.name);
        }
        EnvCommand::Get { app, key } => {
            let app = find_app(api, &app).await?;
            let var: RevealedEnvVar = api
                .post(
                    &format!(
                        "/api/v1/apps/{}/env/{}/reveal",
                        app.id,
                        client::segment(&key)
                    ),
                    None::<&()>,
                )
                .await?;
            println!("{}", var.value);
        }
        EnvCommand::Rm { app, key } => {
            let app = find_app(api, &app).await?;
            api.delete(&format!(
                "/api/v1/apps/{}/env/{}",
                app.id,
                client::segment(&key)
            ))
            .await?;
            println!("Deleted {key}");
        }
    }
    Ok(())
}

/// Resolves `name`, `project/name` or an id to one app.
pub(crate) async fn find_app(api: &Client, wanted: &str) -> Result<App> {
    let apps: Vec<App> = api.get("/api/v1/apps").await?;
    let (project, name) = match wanted.split_once('/') {
        Some((p, n)) => (Some(p), n),
        None => (None, wanted),
    };
    let mut hits: Vec<App> = apps
        .into_iter()
        .filter(|a| {
            a.id == wanted
                || ((a.name.eq_ignore_ascii_case(name) || a.slug == name)
                    && project.is_none_or(|p| a.project_name.eq_ignore_ascii_case(p)))
        })
        .collect();
    match hits.len() {
        0 => bail!("no app called {wanted}. List them with: rig apps"),
        1 => Ok(hits.remove(0)),
        _ => {
            let names: Vec<String> = hits
                .iter()
                .map(|a| format!("{}/{}", a.project_name, a.name))
                .collect();
            bail!(
                "{wanted} matches several apps: {}. Use project/app",
                names.join(", ")
            )
        }
    }
}

async fn after_enqueue(
    api: &Client,
    app: &App,
    dep: Deployment,
    no_follow: bool,
    json: bool,
) -> Result<()> {
    if json {
        return print_json(&dep);
    }
    if no_follow {
        println!("Queued deployment {} for {}", short(&dep.id), app.name);
        return Ok(());
    }
    eprintln!("Deploying {} ({})", app.name, short(&dep.id));
    let mut result = String::new();
    api.events(
        &format!("/api/v1/deployments/{}/logs", dep.id),
        |event, data| match event {
            "log" => print_log(data),
            "end" => result = data.trim_matches('"').to_string(),
            _ => {}
        },
    )
    .await?;
    match result.as_str() {
        "running" => {
            let app: App = api.get(&format!("/api/v1/apps/{}", app.id)).await?;
            match app.domains.first() {
                Some(domain) => println!("Live at {domain}"),
                None => println!("Live. Add a domain to reach it from outside the server"),
            }
            Ok(())
        }
        "" => bail!(
            "the log stream ended before the deployment finished. Check: rig deployments {}",
            app.name
        ),
        other => bail!("deployment {other}"),
    }
}

fn print_log(data: &str) {
    match serde_json::from_str::<LogLine>(data) {
        Ok(line) if line.stream == "stderr" => eprintln!("{}", line.line),
        Ok(line) => println!("{}", line.line),
        Err(_) => println!("{data}"),
    }
}

/// The wire name of an enum value, e.g. `running`.
fn label<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => s,
        Ok(other) => other.to_string(),
        Err(_) => String::new(),
    }
}

pub(crate) fn short(id: &str) -> String {
    let n = id.chars().count();
    id.chars().skip(n.saturating_sub(8)).collect()
}

fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn print_json<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn table(headers: &[&str], rows: Vec<Vec<String>>) {
    if rows.is_empty() {
        return;
    }
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }
    let line = |cells: Vec<&str>| {
        let mut out = String::new();
        for (i, cell) in cells.iter().enumerate() {
            if i + 1 == cells.len() {
                out.push_str(cell);
            } else {
                out.push_str(&format!("{cell:<width$}  ", width = widths[i]));
            }
        }
        println!("{}", out.trim_end());
    };
    line(headers.to_vec());
    for row in &rows {
        line(row.iter().map(String::as_str).collect());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(1536), "1.5 KB");
        assert_eq!(short("0123456789abcdef"), "89abcdef");
        assert_eq!(short("abc"), "abc");
        assert_eq!(label(&DeploymentStatus::Running), "running");
    }

    #[test]
    fn cli_parses() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }
}
