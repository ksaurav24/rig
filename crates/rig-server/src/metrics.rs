//! Resource metrics: a background sampler and the queries that read it back.
//!
//! One row per scope per sample (`host`, `app:<id>`, `db:<id>`), kept for a
//! day. That is a few thousand small rows per scope, so SQLite is plenty and
//! there is no separate time-series store to run.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::Result;
use futures_util::stream::{self, StreamExt};
use rig_api::{MetricPoint, MetricRange, MetricSeries};
use sysinfo::{Disks, Networks, System};

use crate::state::{AppState, SystemSnapshot};

pub const SAMPLE_EVERY: Duration = Duration::from_secs(30);
/// Cadence for the live `/api/v1/system` snapshot cache.
pub const SYSTEM_POLL_EVERY: Duration = Duration::from_secs(10);
/// Maximum age before an on-demand `/api/v1/system` call triggers a refresh
/// (primarily useful in tests or before the first background tick finishes).
pub const SYSTEM_CACHE_TTL: Duration = Duration::from_secs(15);
const RETENTION_SECS: i64 = 24 * 60 * 60;
/// Prune once an hour's worth of samples has been written.
const PRUNE_EVERY_SAMPLES: u64 = 120;
/// A container whose stats do not come back in time is skipped for this round.
const USAGE_TIMEOUT: Duration = Duration::from_secs(8);
const STATS_CONCURRENCY: usize = 8;

pub const HOST: &str = "host";

pub fn app_scope(id: &str) -> String {
    format!("app:{id}")
}

pub fn database_scope(id: &str) -> String {
    format!("db:{id}")
}

#[derive(Debug, Clone, Copy)]
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

/// Read `(disk_used, disk_total)` for the filesystem holding `data_dir` without
/// touching CPU or network samplers.
pub fn sample_disk(data_dir: &Path) -> (u64, u64) {
    let data_dir = data_dir
        .canonicalize()
        .unwrap_or_else(|_| data_dir.to_path_buf());
    let disks = Disks::new_with_refreshed_list().ok();
    let disk = disks.as_ref().and_then(|disks| {
        disks
            .iter()
            .filter(|d| data_dir.starts_with(d.mount_point()))
            .max_by_key(|d| d.mount_point().as_os_str().len())
    });
    disk.map(|d| {
        (
            d.total_space().saturating_sub(d.available_space()),
            d.total_space(),
        )
    })
    .unwrap_or((0, 0))
}

/// Stateful host sampler owning its own `sysinfo::System` and `Networks`
/// instances so its CPU delta interval is never reset by other callers.
struct HostSampler {
    sys: System,
    nets: Option<Networks>,
    data_dir: PathBuf,
}

impl HostSampler {
    fn new(data_dir: &Path, include_nets: bool) -> Result<Self> {
        let mut sys = System::new().map_err(|e| anyhow::anyhow!("reading host usage: {e}"))?;
        // Take an initial baseline so the next sample reflects the true interval.
        sys.refresh_cpu_usage();
        let nets = include_nets.then(|| Networks::new().ok()).flatten();
        let data_dir = data_dir
            .canonicalize()
            .unwrap_or_else(|_| data_dir.to_path_buf());
        Ok(Self {
            sys,
            nets,
            data_dir,
        })
    }

    fn sample(&mut self) -> HostUsage {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
        let (disk_used, disk_total) = sample_disk(&self.data_dir);
        let (net_rx, net_tx) = self.nets.as_mut().map_or((0, 0), |nets| {
            nets.refresh(true);
            nets.iter()
                .filter(|(name, _)| !is_virtual_interface(&name.to_string_lossy()))
                .fold((0u64, 0u64), |(rx, tx), (_, n)| {
                    (
                        rx.saturating_add(n.total_received()),
                        tx.saturating_add(n.total_transmitted()),
                    )
                })
        });
        HostUsage {
            net_rx,
            net_tx,
            cpu_percent: self.sys.global_cpu_usage(),
            memory_used: self.sys.used_memory(),
            memory_total: self.sys.total_memory(),
            disk_used,
            disk_total,
        }
    }
}

/// Refreshes the cached `/api/v1/system` snapshot once, using `host_sampler`
/// when provided (background loop) or a fresh one-shot sampler on demand.
async fn refresh_system_snapshot(
    state: &AppState,
    host_sampler: Option<Arc<Mutex<HostSampler>>>,
) -> Result<SystemSnapshot> {
    let data_dir = state.config.data_dir.clone();
    let usage = tokio::task::spawn_blocking(move || -> Result<HostUsage> {
        if let Some(sampler) = host_sampler {
            let mut guard = sampler.lock().unwrap_or_else(|p| p.into_inner());
            Ok(guard.sample())
        } else {
            let mut s = HostSampler::new(&data_dir, false)?;
            Ok(s.sample())
        }
    });
    let engine = tokio::time::timeout(Duration::from_secs(3), state.runtime.info());
    let (usage, engine, proxy_ready) = tokio::join!(usage, engine, state.proxy.ready());
    let usage = usage.map_err(|e| anyhow::anyhow!("sampling host usage: {e}"))??;
    let engine = engine.ok().and_then(Result::ok);
    let snap = SystemSnapshot {
        engine,
        proxy_ready,
        cpu_percent: usage.cpu_percent,
        memory_used: usage.memory_used,
        memory_total: usage.memory_total,
        disk_used: usage.disk_used,
        disk_total: usage.disk_total,
        sampled_at: Instant::now(),
    };
    *state
        .caches
        .system
        .write()
        .unwrap_or_else(|p| p.into_inner()) = Some(snap.clone());
    Ok(snap)
}

/// Reads the cached `/api/v1/system` snapshot, performing a single-flight
/// refresh only when the cache is empty or older than [`SYSTEM_CACHE_TTL`].
pub async fn cached_system_snapshot(state: &AppState) -> Result<SystemSnapshot> {
    if let Some(snap) = state
        .caches
        .system
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        && snap.sampled_at.elapsed() < SYSTEM_CACHE_TTL
    {
        return Ok(snap.clone());
    }
    let _guard = state.caches.system_refresh.lock().await;
    if let Some(snap) = state
        .caches
        .system
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        && snap.sampled_at.elapsed() < SYSTEM_CACHE_TTL
    {
        return Ok(snap.clone());
    }
    refresh_system_snapshot(state, None).await
}

/// Keeps the `/api/v1/system` snapshot warm in the background using its own
/// dedicated `sysinfo::System` instance so it never perturbs the 30s metrics
/// sampler's CPU delta window.
pub async fn system_poller(state: AppState) {
    let data_dir = state.config.data_dir.clone();
    let host_sampler = match tokio::task::spawn_blocking(move || HostSampler::new(&data_dir, false))
        .await
    {
        Ok(Ok(s)) => Some(Arc::new(Mutex::new(s))),
        Ok(Err(e)) => {
            tracing::warn!("initialising system snapshot sampler: {e:#}");
            None
        }
        Err(e) => {
            tracing::warn!("initialising system snapshot sampler: {e}");
            None
        }
    };
    // Prime immediately so the very first `/api/v1/system` request hits cache.
    if let Err(e) = refresh_system_snapshot(&state, host_sampler.clone()).await {
        tracing::debug!("priming system snapshot: {e:#}");
    }
    let mut tick = tokio::time::interval_at(
        tokio::time::Instant::now() + SYSTEM_POLL_EVERY,
        SYSTEM_POLL_EVERY,
    );
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        if let Err(e) = refresh_system_snapshot(&state, host_sampler.clone()).await {
            tracing::debug!("refreshing system snapshot: {e:#}");
        }
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Samples the host and every running container until the process exits.
pub async fn sampler(state: AppState) {
    // Dedicated HostSampler so CPU usage always spans the true 30-second window.
    let data_dir = state.config.data_dir.clone();
    let host_sampler = match tokio::task::spawn_blocking(move || HostSampler::new(&data_dir, true))
        .await
    {
        Ok(Ok(s)) => Some(Arc::new(Mutex::new(s))),
        Ok(Err(e)) => {
            tracing::warn!("initialising metrics host sampler: {e:#}");
            None
        }
        Err(e) => {
            tracing::warn!("initialising metrics host sampler: {e}");
            None
        }
    };

    let mut tick = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_secs(2),
        SAMPLE_EVERY,
    );
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut samples: u64 = 0;
    let mut counters = Counters::new();
    loop {
        tick.tick().await;
        if let Err(e) = collect(&state, host_sampler.as_ref(), &mut counters).await {
            tracing::warn!("collecting metrics: {e:#}");
        }
        samples += 1;
        if samples % PRUNE_EVERY_SAMPLES == 1 {
            let prune =
                sqlx::query("DELETE FROM metrics WHERE ts < ?").bind(now() - RETENTION_SECS);
            if let Err(e) = prune.execute(&state.db).await {
                tracing::warn!("pruning metrics: {e}");
            }
            crate::auth::prune_expired(&state.db).await;
        }
    }
}

/// Last cumulative network and CPU counters seen per scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Counter {
    ts: i64,
    rx: u64,
    tx: u64,
    cpu_total_ns: Option<u64>,
    cpu_system_ns: Option<u64>,
}

type Counters = HashMap<String, Counter>;

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

/// Computes percent-of-one-core CPU utilization from two one-shot cumulative
/// container stats readings, falling back to `fallback` on the first sample
/// or counter reset (container restart).
fn cpu_percent_delta(
    previous: Option<Counter>,
    cur_total_ns: Option<u64>,
    cur_system_ns: Option<u64>,
    online_cpus: Option<u32>,
    fallback: f64,
) -> f64 {
    let Some(prev) = previous else {
        return fallback;
    };
    let (Some(prev_total), Some(prev_sys), Some(total), Some(sys), Some(cpus)) = (
        prev.cpu_total_ns,
        prev.cpu_system_ns,
        cur_total_ns,
        cur_system_ns,
        online_cpus.filter(|&c| c > 0),
    ) else {
        return fallback;
    };
    let (Some(spent), Some(elapsed)) = (total.checked_sub(prev_total), sys.checked_sub(prev_sys))
    else {
        return fallback;
    };
    if elapsed == 0 {
        return fallback;
    }
    spent as f64 / elapsed as f64 * f64::from(cpus) * 100.0
}

async fn collect(
    state: &AppState,
    host_sampler: Option<&Arc<Mutex<HostSampler>>>,
    counters: &mut Counters,
) -> Result<()> {
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

    let sampler_clone = host_sampler.cloned();
    let data_dir = state.config.data_dir.clone();
    let host = tokio::task::spawn_blocking(move || -> Result<HostUsage> {
        if let Some(sampler) = sampler_clone {
            let mut guard = sampler.lock().unwrap_or_else(|p| p.into_inner());
            Ok(guard.sample())
        } else {
            let mut s = HostSampler::new(&data_dir, true)?;
            Ok(s.sample())
        }
    });
    let container_targets: Vec<(usize, String)> = containers
        .iter()
        .enumerate()
        .map(|(idx, (_, container))| (idx, container.clone()))
        .collect();
    let usages = stream::iter(container_targets)
        .map(|(idx, container)| {
            let runtime = state.runtime.clone();
            async move {
                (
                    idx,
                    tokio::time::timeout(USAGE_TIMEOUT, runtime.usage(&container)).await,
                )
            }
        })
        .buffer_unordered(STATS_CONCURRENCY)
        .collect::<Vec<_>>();
    let (host, usages) = tokio::join!(host, usages);

    struct Row<'a> {
        scope: &'a str,
        cpu: f64,
        cpu_total_ns: Option<u64>,
        cpu_system_ns: Option<u64>,
        online_cpus: Option<u32>,
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
                cpu_total_ns: None,
                cpu_system_ns: None,
                online_cpus: None,
                memory: h.memory_used,
                total: h.memory_total,
                rx: h.net_rx,
                tx: h.net_tx,
            });
        }
        Ok(Err(e)) => tracing::warn!("sampling the host: {e:#}"),
        Err(e) => tracing::warn!("sampling the host: {e}"),
    }
    for (idx, usage) in usages {
        // Timeouts, engine errors and containers that just stopped leave a gap.
        if let Ok(Ok(Some(u))) = usage {
            let (scope, _) = &containers[idx];
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
                cpu_total_ns: u.cpu_total_ns,
                cpu_system_ns: u.cpu_system_ns,
                online_cpus: u.online_cpus,
                memory: u.memory_bytes,
                total: limit,
                rx: u.net_rx,
                tx: u.net_tx,
            });
        }
    }

    // Scopes that left (stopped apps, deleted databases) are forgotten.
    let mut next = Counters::with_capacity(rows.len());
    let rows: Vec<(Row, f64, Option<f64>, Option<f64>)> = rows
        .into_iter()
        .map(|r| {
            let prev = counters.get(r.scope).copied();
            let (rx, tx) = throughput(prev.map(|c| (c.ts, c.rx, c.tx)), ts, r.rx, r.tx);
            let cpu = if r.scope == HOST {
                r.cpu
            } else {
                cpu_percent_delta(prev, r.cpu_total_ns, r.cpu_system_ns, r.online_cpus, r.cpu)
            };
            next.insert(
                r.scope.to_owned(),
                Counter {
                    ts,
                    rx: r.rx,
                    tx: r.tx,
                    cpu_total_ns: r.cpu_total_ns,
                    cpu_system_ns: r.cpu_system_ns,
                },
            );
            (r, cpu, rx, tx)
        })
        .collect();
    *counters = next;

    let mut tx = state.db.begin().await?;
    for (r, cpu, rx, sent) in rows {
        sqlx::query(
            "INSERT OR REPLACE INTO metrics (ts, scope, cpu_pct, mem_bytes, mem_total, net_rx, net_tx)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(ts)
        .bind(r.scope)
        .bind(cpu)
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
    fn cpu_delta_computes_one_shot_percent_and_survives_restarts() {
        // First sample falls back to engine-reported cpu_percent (0.0 for one-shot).
        assert_eq!(
            cpu_percent_delta(None, Some(1_000), Some(10_000), Some(4), 0.0),
            0.0
        );
        let prev = Counter {
            ts: 100,
            rx: 0,
            tx: 0,
            cpu_total_ns: Some(1_000_000),
            cpu_system_ns: Some(10_000_000),
        };
        // 25% of 4 cores = 100% of one core.
        let pct = cpu_percent_delta(
            Some(prev),
            Some(3_500_000),
            Some(20_000_000),
            Some(4),
            0.0,
        );
        assert!((pct - 100.0).abs() < 1e-6, "got {pct}");
        // Counter reset after container restart falls back cleanly without underflow.
        assert_eq!(
            cpu_percent_delta(Some(prev), Some(500_000), Some(20_000_000), Some(4), 0.0),
            0.0
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
