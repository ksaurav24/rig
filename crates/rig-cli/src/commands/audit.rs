//! `rig audit [--limit N] [--action A] [--actor U]`.

use clap::Args;

use crate::api::Client;
use crate::error::CliResult;
use crate::output::{OutputOpts, print_names, print_structured, relative_time, table};

#[derive(Args)]
pub struct AuditArgs {
    /// Maximum number of entries to show.
    #[arg(long)]
    pub limit: Option<usize>,
    /// Filter by action prefix/substring (case-insensitive).
    #[arg(long)]
    pub action: Option<String>,
    /// Filter by actor substring (case-insensitive).
    #[arg(long)]
    pub actor: Option<String>,
}

pub async fn run(api: &Client, args: AuditArgs, out: OutputOpts) -> CliResult<()> {
    let mut entries = api.list_audit().await?;
    if let Some(action) = args.action.as_deref().map(str::to_lowercase) {
        entries.retain(|e| e.action.to_lowercase().contains(&action));
    }
    if let Some(actor) = args.actor.as_deref().map(str::to_lowercase) {
        entries.retain(|e| e.actor.to_lowercase().contains(&actor));
    }
    if let Some(limit) = args.limit {
        entries.truncate(limit);
    }

    if out.is_structured() {
        return print_structured(&entries, out.format);
    }
    if out.is_name() {
        let names: Vec<String> = entries.iter().map(|e| e.id.clone()).collect();
        print_names(&names);
        return Ok(());
    }
    let rows = entries
        .iter()
        .map(|e| {
            vec![
                relative_time(&e.created_at),
                e.actor.clone(),
                e.action.clone(),
                e.target.clone().unwrap_or_default(),
                e.detail.clone().unwrap_or_default(),
            ]
        })
        .collect();
    table(
        &["WHEN", "ACTOR", "ACTION", "TARGET", "DETAIL"],
        rows,
        Some("No audit entries found."),
        out.wide,
    );
    Ok(())
}
