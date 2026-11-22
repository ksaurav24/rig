//! Database listing, resolution, and wait helpers shared between CLI and MCP.

use std::time::{Duration, Instant};

use rig_api::{Backup, BackupStatus, Database, DatabaseStatus};

use crate::api::Client;
use crate::error::{CliError, CliResult};
use crate::ops::apps::looks_like_uuid;
use crate::ops::projects::resolve_project;

/// Lists databases, filtering by project server-side when requested.
pub async fn list_databases(api: &Client, project: Option<&str>) -> CliResult<Vec<Database>> {
    let Some(wanted) = project.map(str::trim).filter(|p| !p.is_empty()) else {
        return api.list_databases_in(None).await;
    };
    if looks_like_uuid(wanted) {
        return api.list_databases_in(Some(wanted)).await;
    }
    match resolve_project(api, wanted).await {
        Ok(proj) => api.list_databases_in(Some(&proj.id)).await,
        Err(CliError::NotFound(_)) => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

/// Resolves `name`, `project/name`, or database id to a single database.
pub async fn resolve_database(api: &Client, wanted: &str) -> CliResult<Database> {
    let dbs = api.list_databases().await?;
    match_database(dbs, wanted.trim())
}

pub fn match_database(dbs: Vec<Database>, wanted: &str) -> CliResult<Database> {
    let (project, name) = match wanted.split_once('/') {
        Some((p, n)) => (Some(p), n),
        None => (None, wanted),
    };
    let mut hits: Vec<Database> = dbs
        .into_iter()
        .filter(|d| {
            d.id == wanted
                || ((d.name.eq_ignore_ascii_case(name) || d.host.eq_ignore_ascii_case(name))
                    && project.is_none_or(|p| {
                        d.project_name.eq_ignore_ascii_case(p) || d.project_id == p
                    }))
        })
        .collect();
    match hits.len() {
        0 => Err(CliError::not_found(format!(
            "no database called {wanted}. List them with: rig db ls"
        ))),
        1 => Ok(hits.remove(0)),
        _ => {
            let names: Vec<String> = hits
                .iter()
                .map(|d| format!("{}/{}", d.project_name, d.name))
                .collect();
            Err(CliError::message(format!(
                "{wanted} matches several databases: {}. Use project/db",
                names.join(", ")
            )))
        }
    }
}

/// Polls a database until it leaves `Creating` (or times out).
pub async fn wait_database_ready(
    api: &Client,
    db_id: &str,
    timeout: Duration,
) -> CliResult<Database> {
    let deadline = Instant::now() + timeout;
    loop {
        let db = api.get_database(db_id).await?;
        if db.status != DatabaseStatus::Creating {
            return Ok(db);
        }
        if Instant::now() >= deadline {
            return Err(CliError::network(format!(
                "timed out waiting for database {} to become ready",
                db.name
            )));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Polls a backup until it leaves `Running` (or times out).
pub async fn wait_backup_done(
    api: &Client,
    db_id: &str,
    backup_id: &str,
    timeout: Duration,
) -> CliResult<Backup> {
    let deadline = Instant::now() + timeout;
    loop {
        let list = api.list_backups(db_id).await?;
        let Some(found) = list.into_iter().find(|b| b.id == backup_id) else {
            return Err(CliError::not_found(format!("backup {backup_id} disappeared")));
        };
        if found.status != BackupStatus::Running {
            return Ok(found);
        }
        if Instant::now() >= deadline {
            return Err(CliError::network(format!(
                "timed out waiting for backup {backup_id} to finish"
            )));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
