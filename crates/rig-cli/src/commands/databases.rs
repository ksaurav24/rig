//! `rig db [ls|engines|create|show|start|stop|rm|url|connect]` (aliased as `rig databases`).

use std::time::Duration;

use clap::{Args, Subcommand, ValueEnum};
use rig_api::{CreateDatabaseRequest, DatabaseEngine};
use serde_json::json;

use crate::api::Client;
use crate::error::{CliError, CliResult};
use crate::ops;
use crate::output::{
    OutputOpts, confirm, label, print_names, print_structured, relative_time, style_status, table,
};

#[derive(Args)]
pub struct DbArgs {
    /// Filter databases by project (name or id).
    #[arg(long)]
    pub project: Option<String>,
    #[command(subcommand)]
    pub command: Option<DbCommand>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum EngineArg {
    Postgres,
    Mysql,
    Mariadb,
    Redis,
    Mongodb,
}

impl From<EngineArg> for DatabaseEngine {
    fn from(v: EngineArg) -> Self {
        match v {
            EngineArg::Postgres => DatabaseEngine::Postgres,
            EngineArg::Mysql => DatabaseEngine::Mysql,
            EngineArg::Mariadb => DatabaseEngine::Mariadb,
            EngineArg::Redis => DatabaseEngine::Redis,
            EngineArg::Mongodb => DatabaseEngine::Mongodb,
        }
    }
}

#[derive(Subcommand)]
pub enum DbCommand {
    /// List managed databases.
    Ls {
        /// Only databases in this project (name or id).
        #[arg(long)]
        project: Option<String>,
    },
    /// List supported database engines and versions.
    Engines,
    /// Create and start a managed database.
    Create {
        /// Database name.
        name: String,
        /// Project name, slug, or id.
        #[arg(long)]
        project: String,
        /// Database engine.
        #[arg(long, value_enum)]
        engine: EngineArg,
        /// Engine version (defaults to newest supported).
        #[arg(long)]
        version: Option<String>,
        /// Wait until the database finishes starting.
        #[arg(long)]
        wait: bool,
    },
    /// Show details for one database.
    Show {
        /// Database name, `project/db`, or id.
        database: String,
    },
    /// Start a stopped or failed database.
    Start {
        /// Database name, `project/db`, or id.
        database: String,
        /// Wait until the database finishes starting.
        #[arg(long)]
        wait: bool,
    },
    /// Stop a running database (its data volume is kept).
    Stop {
        /// Database name, `project/db`, or id.
        database: String,
    },
    /// Delete a database and all of its data.
    Rm {
        /// Database name, `project/db`, or id.
        database: String,
    },
    /// Reveal the connection URL (or password) for a database (recorded in the audit log).
    Url {
        /// Database name, `project/db`, or id.
        database: String,
        /// Print only the password instead of the full URL.
        #[arg(long)]
        password: bool,
    },
    /// Open an interactive client (`psql`, `mysql`, `redis-cli`, `mongosh`) to a running database.
    Connect {
        /// Database name, `project/db`, or id.
        database: String,
        /// Print the client command instead of executing it.
        #[arg(long)]
        print: bool,
    },
}

pub async fn run(api: &Client, args: DbArgs, out: OutputOpts) -> CliResult<()> {
    match args.command {
        None => list(api, args.project.as_deref(), out).await,
        Some(DbCommand::Ls { project }) => {
            list(api, project.as_deref().or(args.project.as_deref()), out).await
        }
        Some(DbCommand::Engines) => {
            let engines = api.list_database_engines().await?;
            if out.is_structured() {
                return print_structured(&engines, out.format);
            }
            if out.is_name() {
                let names: Vec<String> = engines.iter().map(|e| label(&e.engine)).collect();
                print_names(&names);
                return Ok(());
            }
            let rows = engines
                .iter()
                .map(|e| {
                    vec![
                        label(&e.engine),
                        e.label.clone(),
                        e.versions.join(", "),
                        e.port.to_string(),
                    ]
                })
                .collect();
            table(&["ENGINE", "LABEL", "VERSIONS", "PORT"], rows, None, out.wide);
            Ok(())
        }
        Some(DbCommand::Create {
            name,
            project,
            engine,
            version,
            wait,
        }) => {
            let proj = ops::projects::resolve_project(api, &project).await?;
            let mut db = api
                .create_database(&CreateDatabaseRequest {
                    project_id: proj.id,
                    name,
                    engine: engine.into(),
                    version,
                })
                .await?;
            if wait {
                db = ops::databases::wait_database_ready(api, &db.id, Duration::from_secs(180))
                    .await?;
            }
            if out.is_structured() {
                return print_structured(&db, out.format);
            }
            if out.is_name() {
                println!("{}/{}", db.project_name, db.name);
                return Ok(());
            }
            println!(
                "Created database {}/{} ({} {}, {})",
                db.project_name,
                db.name,
                label(&db.engine),
                db.version,
                style_status(&label(&db.status))
            );
            Ok(())
        }
        Some(DbCommand::Show { database }) => {
            let db = ops::databases::resolve_database(api, &database).await?;
            let db = api.get_database(&db.id).await?;
            if out.is_structured() {
                return print_structured(&db, out.format);
            }
            if out.is_name() {
                println!("{}/{}", db.project_name, db.name);
                return Ok(());
            }
            println!("{}/{} ({})", db.project_name, db.name, db.id);
            println!("Engine      {} {}", label(&db.engine), db.version);
            println!("Status      {}", style_status(&label(&db.status)));
            println!("Address     {}:{}", db.host, db.port);
            if let Some(hp) = db.host_port {
                println!("Loopback    127.0.0.1:{hp}");
            }
            println!("Database    {}", db.db_name);
            println!("Username    {}", db.username);
            println!("Created     {} ({})", db.created_at, relative_time(&db.created_at));
            if let Some(err) = &db.error {
                println!("Error       {err}");
            }
            Ok(())
        }
        Some(DbCommand::Start { database, wait }) => {
            let db = ops::databases::resolve_database(api, &database).await?;
            let mut started = api.start_database(&db.id).await?;
            if wait {
                started =
                    ops::databases::wait_database_ready(api, &db.id, Duration::from_secs(180))
                        .await?;
            }
            if out.is_structured() {
                return print_structured(&started, out.format);
            }
            if out.is_name() {
                println!("{}", started.name);
                return Ok(());
            }
            println!(
                "Started {} ({})",
                started.name,
                style_status(&label(&started.status))
            );
            Ok(())
        }
        Some(DbCommand::Stop { database }) => {
            let db = ops::databases::resolve_database(api, &database).await?;
            let stopped = api.stop_database(&db.id).await?;
            if out.is_structured() {
                return print_structured(&stopped, out.format);
            }
            if out.is_name() {
                println!("{}", stopped.name);
                return Ok(());
            }
            println!("Stopped {}", stopped.name);
            Ok(())
        }
        Some(DbCommand::Rm { database }) => {
            let db = ops::databases::resolve_database(api, &database).await?;
            confirm(
                &format!(
                    "Delete database {}/{} and ALL of its data and backups?",
                    db.project_name, db.name
                ),
                out.yes,
            )?;
            api.delete_database(&db.id).await?;
            if out.is_structured() {
                return print_structured(
                    &json!({
                        "deleted": format!("{}/{}", db.project_name, db.name),
                        "id": db.id,
                    }),
                    out.format,
                );
            }
            if out.is_name() {
                println!("{}/{}", db.project_name, db.name);
                return Ok(());
            }
            println!("Deleted {}/{}", db.project_name, db.name);
            Ok(())
        }
        Some(DbCommand::Url { database, password }) => {
            let db = ops::databases::resolve_database(api, &database).await?;
            let creds = api.reveal_database(&db.id).await?;
            if out.is_structured() {
                return print_structured(&creds, out.format);
            }
            if password {
                println!("{}", creds.password);
            } else {
                println!("{}", creds.url);
            }
            Ok(())
        }
        Some(DbCommand::Connect { database, print }) => {
            let db = ops::databases::resolve_database(api, &database).await?;
            let Some(host_port) = db.host_port else {
                return Err(CliError::conflict(format!(
                    "{} has no loopback port exposed (status: {}). Start it first",
                    db.name,
                    label(&db.status)
                )));
            };
            let creds = api.reveal_database(&db.id).await?;
            let local_url = rewrite_url_for_loopback(&creds.url, &db.host, db.port, host_port);
            let (program, cmd_args, env_vars) = match db.engine {
                DatabaseEngine::Postgres => ("psql", vec![local_url.clone()], vec![]),
                DatabaseEngine::Mysql => (
                    "mysql",
                    vec![
                        "-h".into(),
                        "127.0.0.1".into(),
                        "-P".into(),
                        host_port.to_string(),
                        "-u".into(),
                        db.username.clone(),
                        db.db_name.clone(),
                    ],
                    vec![("MYSQL_PWD", creds.password.clone())],
                ),
                DatabaseEngine::Mariadb => (
                    "mariadb",
                    vec![
                        "-h".into(),
                        "127.0.0.1".into(),
                        "-P".into(),
                        host_port.to_string(),
                        "-u".into(),
                        db.username.clone(),
                        db.db_name.clone(),
                    ],
                    vec![("MYSQL_PWD", creds.password.clone())],
                ),
                DatabaseEngine::Redis => (
                    "redis-cli",
                    vec![
                        "-h".into(),
                        "127.0.0.1".into(),
                        "-p".into(),
                        host_port.to_string(),
                    ],
                    vec![("REDISCLI_AUTH", creds.password.clone())],
                ),
                DatabaseEngine::Mongodb => ("mongosh", vec![local_url.clone()], vec![]),
            };
            if out.is_structured() {
                return print_structured(
                    &json!({
                        "database": db.name,
                        "engine": db.engine,
                        "loopback_port": host_port,
                        "program": program,
                        "url": local_url,
                    }),
                    out.format,
                );
            }
            if print {
                println!("{program} {}", cmd_args.join(" "));
                return Ok(());
            }
            let status = std::process::Command::new(program)
                .args(&cmd_args)
                .envs(env_vars)
                .status()
                .map_err(|e| {
                    CliError::message(format!(
                        "could not launch `{program}` ({e}). Install `{program}` or use: rig db url {}",
                        db.name
                    ))
                })?;
            if !status.success() {
                return Err(CliError::message(format!(
                    "`{program}` exited with status {}",
                    status.code().unwrap_or(1)
                )));
            }
            Ok(())
        }
    }
}

async fn list(api: &Client, project: Option<&str>, out: OutputOpts) -> CliResult<()> {
    let dbs = ops::databases::list_databases(api, project).await?;
    if out.is_structured() {
        return print_structured(&dbs, out.format);
    }
    if out.is_name() {
        let names: Vec<String> = dbs
            .iter()
            .map(|d| format!("{}/{}", d.project_name, d.name))
            .collect();
        print_names(&names);
        return Ok(());
    }
    let rows = dbs
        .iter()
        .map(|d| {
            vec![
                format!("{}/{}", d.project_name, d.name),
                format!("{} {}", label(&d.engine), d.version),
                style_status(&label(&d.status)),
                format!("{}:{}", d.host, d.port),
            ]
        })
        .collect();
    table(
        &["DATABASE", "ENGINE", "STATUS", "ADDRESS IN PROJECT"],
        rows,
        Some("No databases yet. Create one with: rig db create <name> --project <project> --engine postgres"),
        out.wide,
    );
    Ok(())
}

fn rewrite_url_for_loopback(url: &str, host: &str, port: u16, host_port: u16) -> String {
    let from = format!("@{host}:{port}");
    let to = format!("@127.0.0.1:{host_port}");
    url.replacen(&from, &to, 1)
}
