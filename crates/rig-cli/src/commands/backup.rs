//! `rig backup ls|create|download|restore|rm`.

use std::path::PathBuf;
use std::time::Duration;

use clap::{Args, Subcommand};
use serde_json::json;

use crate::api::Client;
use crate::error::CliResult;
use crate::ops;
use crate::output::{
    OutputOpts, bytes, confirm, label, print_names, print_structured, relative_time, short,
    style_status, table,
};

#[derive(Args)]
pub struct BackupArgs {
    #[command(subcommand)]
    pub command: BackupCommand,
}

#[derive(Subcommand)]
pub enum BackupCommand {
    /// List backups for a database.
    Ls {
        /// Database name, `project/db`, or id.
        database: String,
    },
    /// Take a new backup of a running database.
    Create {
        /// Database name, `project/db`, or id.
        database: String,
        /// Wait until the backup finishes.
        #[arg(long)]
        wait: bool,
    },
    /// Download a backup dump file.
    Download {
        /// Backup id.
        id: String,
        /// Destination file path (defaults to the server-suggested filename in the current directory).
        #[arg(long, short = 'f')]
        out_file: Option<PathBuf>,
    },
    /// Restore a backup into its database, replacing current data.
    Restore {
        /// Backup id.
        id: String,
    },
    /// Delete a backup and its dump file.
    Rm {
        /// Backup id.
        id: String,
    },
}

pub async fn run(api: &Client, args: BackupArgs, out: OutputOpts) -> CliResult<()> {
    match args.command {
        BackupCommand::Ls { database } => {
            let db = ops::databases::resolve_database(api, &database).await?;
            let backups = api.list_backups(&db.id).await?;
            if out.is_structured() {
                return print_structured(&backups, out.format);
            }
            if out.is_name() {
                let names: Vec<String> = backups.iter().map(|b| b.id.clone()).collect();
                print_names(&names);
                return Ok(());
            }
            let rows = backups
                .iter()
                .map(|b| {
                    let size = b
                        .size_bytes
                        .and_then(|s| u64::try_from(s).ok())
                        .map(bytes)
                        .unwrap_or_else(|| "-".into());
                    vec![
                        short(&b.id),
                        style_status(&label(&b.status)),
                        size,
                        if b.restorable { "yes" } else { "no" }.into(),
                        relative_time(&b.created_at),
                        b.id.clone(),
                    ]
                })
                .collect();
            table(
                &["SHORT", "STATUS", "SIZE", "RESTORABLE", "CREATED", "ID"],
                rows,
                Some(&format!(
                    "No backups for {} yet. Take one with: rig backup create {}",
                    db.name, db.name
                )),
                out.wide,
            );
        }
        BackupCommand::Create { database, wait } => {
            let db = ops::databases::resolve_database(api, &database).await?;
            let mut backup = api.create_backup(&db.id).await?;
            if wait {
                backup =
                    ops::databases::wait_backup_done(api, &db.id, &backup.id, Duration::from_secs(300))
                        .await?;
            }
            if out.is_structured() {
                return print_structured(&backup, out.format);
            }
            if out.is_name() {
                println!("{}", backup.id);
                return Ok(());
            }
            println!(
                "Backup {} for {}: {}",
                short(&backup.id),
                db.name,
                style_status(&label(&backup.status))
            );
        }
        BackupCommand::Download { id, out_file } => {
            let (path, written) = api.download_backup(&id, out_file.as_deref()).await?;
            if out.is_structured() {
                return print_structured(
                    &json!({
                        "backup_id": id,
                        "path": path.display().to_string(),
                        "bytes": written,
                    }),
                    out.format,
                );
            }
            if out.is_name() {
                println!("{}", path.display());
                return Ok(());
            }
            println!("Saved {} ({})", path.display(), bytes(written));
        }
        BackupCommand::Restore { id } => {
            confirm(
                &format!(
                    "Restore backup {}? This will REPLACE all current data in the database",
                    short(&id)
                ),
                out.yes,
            )?;
            api.restore_backup(&id).await?;
            if out.is_structured() {
                return print_structured(&json!({ "restored": id }), out.format);
            }
            if out.is_name() {
                println!("{id}");
                return Ok(());
            }
            println!("Restored backup {}", short(&id));
        }
        BackupCommand::Rm { id } => {
            confirm(&format!("Delete backup {}?", short(&id)), out.yes)?;
            api.delete_backup(&id).await?;
            if out.is_structured() {
                return print_structured(&json!({ "deleted": id }), out.format);
            }
            if out.is_name() {
                println!("{id}");
                return Ok(());
            }
            println!("Deleted backup {}", short(&id));
        }
    }
    Ok(())
}
