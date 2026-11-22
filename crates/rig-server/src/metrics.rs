//! Resource metrics: a background sampler and the queries that read it back.
//!
//! One row per scope per sample (`host`, `app:<id>`, `db:<id>`), kept for a
//! day. That is a few thousand small rows per scope, so SQLite is plenty and
//! there is no separate time-series store to run.

use std::{
    collections::HashMap,
    path::Path,
    sync::{Mutex, OnceLock},
    time::Duration,
};

use anyhow::Result;
use futures_util::future::join_all;
use rig_api::{MetricPoint, MetricRange, MetricSeries};
use sysinfo::{Disks, Networks, System};

use crate::state::AppState;

pub const SAMPLE_EVERY: Duration = Duration::from_secs(30);
const RETENTION_SECS: i64 = 24 * 60 * 60;
/// Prune once an hour's worth of samples has been written.
const PRUNE_EVERY_SAMPLES: u64 = 120;
/// A container whose stats do not come back in time is skipped for this round.
const USAGE_TIMEOUT: Duration = Duration::from_secs(8);

pub const HOST: &str = "host";

pub fn app_scope(id: &str) -> String {
    format!("app:{id}")
}

pub fn database_scope(id: &str) -> String {
    format!("db:{id}")
}

pub struct HostUsage {
    pub cpu_percent: f32,
    pub memory_used: u64,
    pub memory_total: u64,
    pub disk_used: u64,
    pub disk_total: u64,
    /// Bytes through the host's real interfaces since boot.
    pub net_rx: u64,
    pub net_tx: u64,
}

/// Loopback and the engine's virtual interfaces carry traffic that also
/// crosses a real interface (or never leaves the host), so they are skipped.
fn is_virtual_interface(name: &str) -> bool {
    name == "lo"
        || [
            "veth", "docker", "br-", "podman", "cni", "virbr", "vnet", "tap",
        ]
        .iter()
        .any(|p| name.starts_with(p))
}

/// Sample the host. CPU usage is measured between calls, so one `System` is
/// kept for the life of the process and the first reading is zero.
pub fn sample_host(data_dir: &Path) -> anyhow::Result<HostUsage> {
    static SYS: OnceLock<Mutex<System>> = OnceLock::new();
    let sys = match SYS.get() {
        Some(sys) => sys,
        None => {
            let fresh = System::new().map_err(|e| anyhow::anyhow!("reading host usage: {e}"))?;
            SYS.get_or_init(|| Mutex::new(fresh))
        }
    };
    let mut sys = sys.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    sys.refresh_cpu_usage();
    sys.refresh_memory();

    // The disk that holds Rig's data: the mount point that is the longest
    // prefix of the data directory.
    let data_dir = data_dir
        .canonicalize()
        .unwrap_or_else(|_| data_dir.to_path_buf());
    // Disk figures are best-effort: without them the rest is still useful.
    let disks = Disks::new_with_refreshed_list().ok();
    let disk = disks.as_ref().and_then(|disks| {
        disks
            .iter()
            .filter(|d| data_dir.starts_with(d.mount_point()))
            .max_by_key(|d| d.mount_point().as_os_str().len())
    });
    let (disk_used, disk_total) = disk
        .map(|d| {
            (
                d.total_space().saturating_sub(d.available_space()),
                d.total_space(),
            )
        })
        .unwrap_or((0, 0));

    // Network figures are best-effort too.
    static NETS: OnceLock<Mutex<Option<Networks>>> = OnceLock::new();
    let (net_rx, net_tx) = {
        let mut nets = NETS
            .get_or_init(|| Mutex::new(Networks::new().ok()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        nets.as_mut().map_or((0, 0), |nets| {
            nets.refresh(true);
            nets.iter()
                .filter(|(name, _)| !is_virtual_interface(&name.to_string_lossy()))
                .fold((0u64, 0u64), |(rx, tx), (_, n)| {
                    (
                        rx.saturating_add(n.total_received()),
                        tx.saturating_add(n.total_transmitted()),
                    )
                })
        })
    };

    Ok(HostUsage {
        net_rx,
        net_tx,
        cpu_percent: sys.global_cpu_usage(),
        memory_used: sys.used_memory(),
        memory_total: sys.total_memory(),
        disk_used,
        disk_total,
    })
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Samples the host and every running container until the process exits.
pub async fn sampler(state: AppState) {
    // CPU use is measured between two readings, so take one to measure from.
    let data_dir = state.config.data_dir.clone();
    let _ = tokio::task::spawn_blocking(move || sample_host(&data_dir)).await;

    let mut tick = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_secs(2),
        SAMPLE_EVERY,
    );
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut samples: u64 = 0;
    let mut counters = Counters::new();
    loop {
        tick.tick().await;
        if let Err(e) = collect(&state, &mut counters).await {
            tracing::warn!("collecting metrics: {e:#}");
        }
        samples += 1;
        if samples % PRUNE_EVERY_SAMPLES == 1 {
            let prune =
                sqlx::query("DELETE FROM metrics WHERE ts < ?").bind(now() - RETENTION_SECS);
            if let Err(e) = prune.execute(&state.db).await {
                tracing::warn!("pruning metrics: {e}");
            }
        }
    }
}

/// Last network counters seen per scope: (unix seconds, received, sent).
type Counters = HashMap<String, (i64, u64, u64)>;

/// Bytes per second since the previous reading of the same counters. `None`
/// for the first reading and when a counter went backwards (a restart).
fn throughput(
    previous: Option<(i64, u64, u64)>,
    ts: i64,
    rx: u64,
    tx: u64,
) -> (Option<f64>, Option<f64>) {
    let Some((then, rx0, tx0)) = previous else {
        return (None, None);
    };
    let secs = ts - then;
    if secs <= 0 || rx < rx0 || tx < tx0 {
        return (None, None);
    }
    let rate = |delta: u64| Some(delta as f64 / secs as f64);
    (rate(rx - rx0), rate(tx - tx0))
}

async fn collect(state: &AppState, counters: &mut Counters) -> Result<()> {
    let ts = now();
    let containers: Vec<(String, String)> = sqlx::query_as(
        "SELECT 'app:' || a.id, d.container_id FROM apps a
           JOIN deployments d ON d.id = a.current_deployment_id
          WHERE a.status = 'running' AND d.container_id IS NOT NULL
         UNION ALL
         SELECT 'db:' || id, container_id FROM databases
          WHERE status = 'running' AND container_id IS NOT NULL",
    )
    .fetch_all(&state.db)
    .await?;

    let data_dir = state.config.data_dir.clone();
    let host = tokio::task::spawn_blocking(move || sample_host(&data_dir));
    let usages = join_all(containers.iter().map(|(_, container)| {
        let runtime = state.runtime.clone();
        async move { tokio::time::timeout(USAGE_TIMEOUT, runtime.usage(container)).await }
    }));
    let (host, usages) = tokio::join!(host, usages);

    struct Row<'a> {
        scope: &'a str,
        cpu: f64,
        memory: u64,
        total: u64,
        rx: u64,
        tx: u64,
    }
    let mut rows: Vec<Row> = Vec::with_capacity(containers.len() + 1);
    let mut host_memory = u64::MAX;
    match host {
        Ok(Ok(h)) => {
            host_memory = h.memory_total;
            if let Some(free) = crate::alerts::disk_warning(h.disk_total, h.disk_used) {
                crate::alerts::notify_all(
                    state,
                    crate::alerts::Alert {
                        event: rig_api::AlertEvent::DiskLow,
                        title: "Disk space is low".into(),
                        detail: format!(
                            "{} MB free on the disk that holds Rig's data. Deploys and backups pause below 1024 MB.",
                            free / (1024 * 1024)
                        ),
                        target: None,
                    },
                );
            }
            rows.push(Row {
                scope: HOST,
                cpu: f64::from(h.cpu_percent),
                memory: h.memory_used,
                total: h.memory_total,
                rx: h.net_rx,
                tx: h.net_tx,
            });
        }
        Ok(Err(e)) => tracing::warn!("sampling the host: {e:#}"),
        Err(e) => tracing::warn!("sampling the host: {e}"),
    }
    for ((scope, _), usage) in containers.iter().zip(usages) {
        // Timeouts, engine errors and containers that just stopped leave a gap.
        if let Ok(Ok(Some(u))) = usage {
            // The engine reports the host's memory as the limit of a container
            // that has none. Store that as "no limit" rather than a fake one.
            let limit = if u.memory_limit >= host_memory {
                0
            } else {
                u.memory_limit
            };
            rows.push(Row {
                scope,
                cpu: u.cpu_percent,
                memory: u.memory_bytes,
                total: limit,
                rx: u.net_rx,
                tx: u.net_tx,
            });
        }
    }

    // Scopes that left (stopped apps, deleted databases) are forgotten.
    let mut next = Counters::with_capacity(rows.len());
    let rows: Vec<(Row, Option<f64>, Option<f64>)> = rows
        .into_iter()
        .map(|r| {
            let (rx, tx) = throughput(counters.get(r.scope).copied(), ts, r.rx, r.tx);
            next.insert(r.scope.to_owned(), (ts, r.rx, r.tx));
            (r, rx, tx)
        })
        .collect();
    *counters = next;

    let mut tx = state.db.begin().await?;
    for (r, rx, sent) in rows {
        sqlx::query(
            "INSERT OR REPLACE INTO metrics (ts, scope, cpu_pct, mem_bytes, mem_total, net_rx, net_tx)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(ts)
        .bind(r.scope)
        .bind(r.cpu)
        .bind(i64::try_from(r.memory).unwrap_or(i64::MAX))
        .bind(i64::try_from(r.total).unwrap_or(i64::MAX))
        .bind(rx)
        .bind(sent)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Seconds covered by a range, and the width of each returned point.
/// Every range comes back as about 120 points.
fn window(range: MetricRange) -> (i64, i64) {
    match range {
        MetricRange::Hour => (60 * 60, 30),
        MetricRange::SixHours => (6 * 60 * 60, 180),
        MetricRange::Day => (24 * 60 * 60, 720),
    }
}

pub async fn series(
    state: &AppState,
    scope: &str,
    range: MetricRange,
) -> Result<MetricSeries, sqlx::Error> {
    let (span, step) = window(range);
    type Bucket = (i64, f64, i64, i64, Option<f64>, Option<f64>);
    let rows: Vec<Bucket> = sqlx::query_as(
        "SELECT (ts / ?1) * ?1 AS bucket, AVG(cpu_pct), CAST(AVG(mem_bytes) AS INTEGER), MAX(mem_total),
                AVG(net_rx), AVG(net_tx)
           FROM metrics WHERE scope = ?2 AND ts >= ?3
          GROUP BY bucket ORDER BY bucket",
    )
    .bind(step)
    .bind(scope)
    .bind(now() - span)
    .fetch_all(&state.db)
    .await?;

    let unsigned = |n: i64| u64::try_from(n).unwrap_or(0);
    Ok(MetricSeries {
        range,
        step_seconds: step as u32,
        memory_total: rows.last().map(|r| unsigned(r.3)).unwrap_or(0),
        points: rows
            .iter()
            .map(|&(ts, cpu_percent, memory, _, rx, tx)| MetricPoint {
                ts,
                cpu_percent,
                memory_bytes: unsigned(memory),
                net_rx_bps: rx,
                net_tx_bps: tx,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn throughput_needs_two_readings_and_survives_resets() {
        assert_eq!(throughput(None, 30, 100, 100), (None, None));
        assert_eq!(
            throughput(Some((0, 0, 30)), 30, 300, 90),
            (Some(10.0), Some(2.0))
        );
        assert_eq!(
            throughput(Some((0, 500, 0)), 30, 100, 10),
            (None, None),
            "counter reset"
        );
        assert_eq!(
            throughput(Some((30, 0, 0)), 30, 1, 1),
            (None, None),
            "no time passed"
        );
    }

    #[test]
    fn virtual_interfaces_are_skipped() {
        for name in ["lo", "veth12ab", "docker0", "br-3f2a", "podman0"] {
            assert!(is_virtual_interface(name), "{name}");
        }
        for name in ["eth0", "enp3s0", "wlan0", "wlp2s0", "tailscale0"] {
            assert!(!is_virtual_interface(name), "{name}");
        }
    }

    #[test]
    fn every_range_is_about_the_same_number_of_points() {
        for range in [MetricRange::Hour, MetricRange::SixHours, MetricRange::Day] {
            let (span, step) = window(range);
            assert_eq!(span / step, 120);
            // Buckets must be whole samples, or some would be emptier than others.
            assert_eq!(step % SAMPLE_EVERY.as_secs() as i64, 0);
        }
    }
}
