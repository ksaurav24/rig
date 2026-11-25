//! `rig snapshot download|inspect|restore` — full portable server snapshots.

use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};
use rig_api::{SnapshotExportRequest, SnapshotRestoreMode, SnapshotRestoreOptions};
use serde_json::json;

use crate::api::Client;
use crate::error::CliResult;
use crate::output::{OutputOpts, bytes, confirm, print_structured, table};

#[derive(Args)]
pub struct SnapshotArgs {
    /// Target a specific Rig server context or URL (overrides default context).
    #[arg(long)]
    pub node: Option<String>,
    #[command(subcommand)]
    pub command: SnapshotCommand,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum CliRestoreMode {
    Merge,
    Replace,
}

impl From<CliRestoreMode> for SnapshotRestoreMode {
    fn from(m: CliRestoreMode) -> Self {
        match m {
            CliRestoreMode::Merge => SnapshotRestoreMode::Merge,
            CliRestoreMode::Replace => SnapshotRestoreMode::Replace,
        }
    }
}

#[derive(Subcommand)]
pub enum SnapshotCommand {
    /// Download a full portable snapshot of the server.
    Download {
        /// Destination file path (defaults to the server-suggested filename).
        #[arg(long, short = 'f')]
        out_file: Option<PathBuf>,
        /// Encrypt the snapshot archive with Argon2id + XChaCha20-Poly1305.
        #[arg(long, env = "RIG_SNAPSHOT_PASSPHRASE")]
        passphrase: Option<String>,
        /// Skip live database dumps (configuration & secrets only).
        #[arg(long)]
        no_databases: bool,
        /// Exclude alert channels & webhook targets.
        #[arg(long)]
        no_alerts: bool,
        /// Include the organization's GitHub App private key & installation.
        #[arg(long)]
        include_github: bool,
    },
    /// Inspect a local snapshot archive against the target server without modifying anything.
    Inspect {
        /// Path to the `.rig-snapshot.tar.zst` or `.enc` file.
        file: PathBuf,
        /// Passphrase for encrypted snapshots.
        #[arg(long, env = "RIG_SNAPSHOT_PASSPHRASE")]
        passphrase: Option<String>,
    },
    /// Restore a local snapshot archive onto the target Rig server.
    Restore {
        /// Path to the `.rig-snapshot.tar.zst` or `.enc` file.
        file: PathBuf,
        /// Merge safely (skip existing names) or replace existing projects/apps/databases.
        #[arg(long, value_enum, default_value = "merge")]
        mode: CliRestoreMode,
        /// Passphrase for encrypted snapshots.
        #[arg(long, env = "RIG_SNAPSHOT_PASSPHRASE")]
        passphrase: Option<String>,
        /// Skip updating organization branding/logo.
        #[arg(long)]
        no_branding: bool,
        /// Do not start databases or restore database dumps.
        #[arg(long)]
        no_databases: bool,
        /// Skip restoring alert channels.
        #[arg(long)]
        no_alerts: bool,
        /// Also restore GitHub App configuration if present in the snapshot.
        #[arg(long)]
        restore_github: bool,
        /// Do not automatically queue deployments for previously running apps.
        #[arg(long)]
        no_deploy: bool,
    },
}

pub async fn run(api: &Client, args: SnapshotArgs, out: OutputOpts) -> CliResult<()> {
    match args.command {
        SnapshotCommand::Download {
            out_file,
            passphrase,
            no_databases,
            no_alerts,
            include_github,
        } => {
            let req = SnapshotExportRequest {
                include_database_dumps: !no_databases,
                include_alerts: !no_alerts,
                include_github,
                passphrase: passphrase.filter(|s| !s.is_empty()),
            };
            let (path, written, encrypted) =
                api.export_snapshot(&req, out_file.as_deref()).await?;
            if out.is_structured() {
                return print_structured(
                    &json!({
                        "path": path.display().to_string(),
                        "bytes": written,
                        "encrypted": encrypted,
                        "server": api.base(),
                    }),
                    out.format,
                );
            }
            if out.is_name() {
                println!("{}", path.display());
                return Ok(());
            }
            let enc = if encrypted { " (encrypted)" } else { "" };
            println!(
                "Saved server snapshot {} ({}){enc}",
                path.display(),
                bytes(written)
            );
        }
        SnapshotCommand::Inspect { file, passphrase } => {
            let preview = api
                .inspect_snapshot(&file, passphrase.as_deref())
                .await?;
            if out.is_structured() {
                return print_structured(&preview, out.format);
            }
            if out.is_name() {
                println!("{}", preview.source_org_name);
                return Ok(());
            }
            println!(
                "Snapshot v{} · Org: {} · Created: {} · Server v{} · Encrypted: {}",
                preview.format_version,
                preview.source_org_name,
                preview.created_at,
                preview.rig_version,
                if preview.encrypted { "yes" } else { "no" },
            );
            let rows = vec![
                vec!["Projects".into(), preview.projects.len().to_string()],
                vec!["Apps".into(), preview.apps.len().to_string()],
                vec!["Databases".into(), preview.databases.len().to_string()],
                vec![
                    "Alert channels".into(),
                    preview.alert_channel_count.to_string(),
                ],
                vec![
                    "GitHub App".into(),
                    if preview.has_github_app { "included" } else { "no" }.into(),
                ],
                vec!["Media files".into(), preview.media_count.to_string()],
            ];
            table(&["RESOURCE", "COUNT"], rows, None, out.wide);
            if !preview.warnings.is_empty() {
                println!("\nWarnings:");
                for w in &preview.warnings {
                    println!("  - {w}");
                }
            }
        }
        SnapshotCommand::Restore {
            file,
            mode,
            passphrase,
            no_branding,
            no_databases,
            no_alerts,
            restore_github,
            no_deploy,
        } => {
            let preview = api
                .inspect_snapshot(&file, passphrase.as_deref())
                .await?;
            let mode_label = match mode {
                CliRestoreMode::Merge => "merge",
                CliRestoreMode::Replace => "REPLACE existing",
            };
            confirm(
                &format!(
                    "Restore snapshot from `{}` ({} projects, {} apps, {} databases) onto {} in {mode_label} mode?",
                    preview.source_org_name,
                    preview.projects.len(),
                    preview.apps.len(),
                    preview.databases.len(),
                    api.base(),
                ),
                out.yes,
            )?;
            let opts = SnapshotRestoreOptions {
                mode: mode.into(),
                restore_branding: !no_branding,
                restore_alerts: !no_alerts,
                restore_github,
                start_databases: !no_databases,
                deploy_running_apps: !no_deploy,
            };
            let report = api
                .restore_snapshot(&file, passphrase.as_deref(), &opts)
                .await?;
            if out.is_structured() {
                return print_structured(&report, out.format);
            }
            if out.is_name() {
                println!("{}", api.base());
                return Ok(());
            }
            println!(
                "Restored onto {}: {} projects (+{} skipped), {} apps (+{} skipped, {} deploys queued), {} databases (+{} skipped, {} dumps restoring), {} alerts",
                api.base(),
                report.projects_created,
                report.projects_skipped,
                report.apps_created,
                report.apps_skipped,
                report.apps_deployments_queued,
                report.databases_created,
                report.databases_skipped,
                report.database_dumps_scheduled,
                report.alerts_created,
            );
            for w in &report.warnings {
                println!("  warning: {w}");
            }
        }
    }
    Ok(())
}
