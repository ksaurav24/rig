//! `rig logs [app] [--tail N] [--no-follow] [--since 10m] [--timestamps] [--grep pat]`.

use clap::Args;
use rig_api::LogLine;

use crate::api::{AppLogQuery, Client};
use crate::error::{CliError, CliResult};
use crate::ops;
use crate::output::{OutputOpts, print_log_opts};

#[derive(Args)]
pub struct LogsArgs {
    /// App name, `project/app`, or id. Defaults to `rig.toml`.
    pub app: Option<String>,
    /// Number of trailing lines to show (default 200).
    #[arg(long, short = 'n')]
    pub tail: Option<u32>,
    /// Print existing log lines and exit instead of streaming live output.
    #[arg(long)]
    pub no_follow: bool,
    /// Only show logs since a relative duration (`10m`, `2h`, `30s`) or RFC 3339 timestamp.
    #[arg(long)]
    pub since: Option<String>,
    /// Prefix each line with its timestamp.
    #[arg(long, short = 't')]
    pub timestamps: bool,
    /// Only show lines containing this text (case-insensitive).
    #[arg(long)]
    pub grep: Option<String>,
}

pub async fn run(api: &Client, args: LogsArgs, out: OutputOpts) -> CliResult<()> {
    let app = ops::apps::resolve_app_or_linked(api, args.app.as_deref()).await?;
    if app.current_deployment_id.is_none() {
        return Err(CliError::message(format!(
            "{} is not running, so it has no logs. Deploy it first",
            app.name
        )));
    }
    let grep_lower = args.grep.as_deref().map(str::to_lowercase);
    let q = AppLogQuery {
        tail: args.tail,
        follow: args.no_follow.then_some(false),
        since: args.since,
        timestamps: args.timestamps.then_some(true),
        grep: args.grep,
    };
    let json_out = out.is_json();
    let show_ts = args.timestamps;

    let stream = api.app_logs_stream(&app.id, &q, |ev| {
        if ev.event != "log" {
            return;
        }
        // Client-side grep fallback in case an older server ignores `?grep=`.
        if let Some(needle) = &grep_lower {
            let text = serde_json::from_str::<LogLine>(&ev.data)
                .map(|l| l.line)
                .unwrap_or_else(|_| ev.data.clone());
            if !text.to_lowercase().contains(needle) {
                return;
            }
        }
        print_log_opts(&ev.data, json_out, show_ts);
    });
    tokio::select! {
        res = stream => res,
        _ = tokio::signal::ctrl_c() => Ok(()),
    }
}
