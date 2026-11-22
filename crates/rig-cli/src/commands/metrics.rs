//! `rig metrics [target] [--range 1h|6h|24h]` and `rig top`.

use std::io::IsTerminal;
use std::time::Duration;

use clap::{Args, ValueEnum};
use rig_api::{MetricRange, MetricSeries};
use serde_json::json;

use crate::api::Client;
use crate::error::{CliError, CliResult};
use crate::ops;
use crate::output::{
    OutputOpts, bytes, label, print_structured, sparkline, style_status, table,
};

#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum RangeArg {
    #[default]
    #[value(name = "1h")]
    Hour,
    #[value(name = "6h")]
    SixHours,
    #[value(name = "24h")]
    Day,
}

impl From<RangeArg> for MetricRange {
    fn from(v: RangeArg) -> Self {
        match v {
            RangeArg::Hour => MetricRange::Hour,
            RangeArg::SixHours => MetricRange::SixHours,
            RangeArg::Day => MetricRange::Day,
        }
    }
}

#[derive(Args)]
pub struct MetricsArgs {
    /// Target app or database (prefix with `db:` for a database). Omit for host metrics.
    pub target: Option<String>,
    /// Treat target as a database instead of an app.
    #[arg(long)]
    pub db: bool,
    /// Time range: `1h`, `6h`, or `24h`.
    #[arg(long, value_enum, default_value_t = RangeArg::Hour)]
    pub range: RangeArg,
}

#[derive(Args)]
pub struct TopArgs {
    /// Print one snapshot and exit instead of refreshing continuously.
    #[arg(long)]
    pub once: bool,
    /// Refresh interval in seconds (default 2).
    #[arg(long, default_value_t = 2)]
    pub interval: u64,
}

pub async fn run_metrics(api: &Client, args: MetricsArgs, out: OutputOpts) -> CliResult<()> {
    let range: MetricRange = args.range.into();
    let (title, series) = match args.target.as_deref() {
        None => ("Host".to_string(), api.host_metrics(range).await?),
        Some(raw) => {
            if args.db || raw.starts_with("db:") {
                let name = raw.strip_prefix("db:").unwrap_or(raw);
                let db = ops::databases::resolve_database(api, name).await?;
                (
                    format!("Database {}/{}", db.project_name, db.name),
                    api.database_metrics(&db.id, range).await?,
                )
            } else {
                match ops::apps::resolve_app(api, raw).await {
                    Ok(app) => (
                        format!("App {}/{}", app.project_name, app.name),
                        api.app_metrics(&app.id, range).await?,
                    ),
                    Err(CliError::NotFound(_)) => {
                        let db = ops::databases::resolve_database(api, raw).await?;
                        (
                            format!("Database {}/{}", db.project_name, db.name),
                            api.database_metrics(&db.id, range).await?,
                        )
                    }
                    Err(e) => return Err(e),
                }
            }
        }
    };

    if out.is_structured() {
        return print_structured(&series, out.format);
    }

    render_series(&title, &series);
    Ok(())
}

fn render_series(title: &str, s: &MetricSeries) {
    println!(
        "{title} · range {} · {} sample(s) (step {}s)",
        label(&s.range),
        s.points.len(),
        s.step_seconds
    );
    if s.points.is_empty() {
        println!("No metric samples collected yet.");
        return;
    }
    let cpus: Vec<f64> = s.points.iter().map(|p| p.cpu_percent).collect();
    let mems: Vec<f64> = s.points.iter().map(|p| p.memory_bytes as f64).collect();
    let last = s.points.last().expect("non-empty");
    let avg_cpu = cpus.iter().sum::<f64>() / cpus.len() as f64;
    let max_cpu = cpus.iter().copied().fold(0.0_f64, f64::max);
    let max_mem = s.points.iter().map(|p| p.memory_bytes).max().unwrap_or(0);
    let mem_limit = if s.memory_total > 0 {
        format!(" / {}", bytes(s.memory_total))
    } else {
        String::new()
    };

    println!(
        "CPU     now {:>5.1}% · avg {:>5.1}% · peak {:>5.1}%  {}",
        last.cpu_percent,
        avg_cpu,
        max_cpu,
        sparkline(&cpus)
    );
    println!(
        "Memory  now {}{} · peak {}  {}",
        bytes(last.memory_bytes),
        mem_limit,
        bytes(max_mem),
        sparkline(&mems)
    );
    if let (Some(rx), Some(tx)) = (last.net_rx_bps, last.net_tx_bps) {
        println!(
            "Net     rx {}/s · tx {}/s",
            bytes(rx.max(0.0) as u64),
            bytes(tx.max(0.0) as u64)
        );
    }
}

pub async fn run_top(api: &Client, args: TopArgs, out: OutputOpts) -> CliResult<()> {
    let single_shot = args.once || out.is_structured() || !std::io::stdout().is_terminal();
    let interval = Duration::from_secs(args.interval.max(1));

    loop {
        let (sys, apps, dbs) = tokio::try_join!(api.system(), api.list_apps(None), api.list_databases())?;
        if out.is_structured() {
            return print_structured(
                &json!({
                    "system": sys,
                    "apps": apps,
                    "databases": dbs,
                }),
                out.format,
            );
        }
        if !single_shot {
            // Clear screen and move cursor home on interactive TTY refreshes.
            print!("\x1b[2J\x1b[H");
        }
        println!(
            "Rig {} · CPU {:.0}% · RAM {} / {} · Disk {} / {} · Apps {}/{}",
            sys.version,
            sys.cpu_percent,
            bytes(sys.memory_used),
            bytes(sys.memory_total),
            bytes(sys.disk_used),
            bytes(sys.disk_total),
            sys.apps_running,
            sys.apps_total
        );
        println!();
        let mut rows: Vec<Vec<String>> = apps
            .iter()
            .map(|a| {
                vec![
                    "app".into(),
                    format!("{}/{}", a.project_name, a.name),
                    style_status(&label(&a.status)),
                    a.memory_mb
                        .map(|m| format!("{m} MB"))
                        .unwrap_or_else(|| "512 MB".into()),
                    a.domains.first().cloned().unwrap_or_default(),
                ]
            })
            .collect();
        for d in &dbs {
            rows.push(vec![
                "db".into(),
                format!("{}/{}", d.project_name, d.name),
                style_status(&label(&d.status)),
                format!("{} {}", label(&d.engine), d.version),
                format!("{}:{}", d.host, d.port),
            ]);
        }
        table(
            &["KIND", "SERVICE", "STATUS", "SPEC", "ADDRESS"],
            rows,
            Some("No apps or databases yet."),
            out.wide,
        );

        if single_shot {
            return Ok(());
        }
        tokio::select! {
            _ = tokio::time::sleep(interval) => {}
            _ = tokio::signal::ctrl_c() => return Ok(()),
        }
    }
}
