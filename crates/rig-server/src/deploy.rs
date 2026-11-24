//! The deployment pipeline.
//!
//! Deployments are rows in SQLite with status `queued`; one worker takes them
//! oldest first. A deployment builds or pulls an image, starts a new
//! container beside the old one, waits for it to answer, switches the proxy,
//! and only then stops the old container. A failed deployment never touches
//! the container that is already serving.

use std::{
    collections::HashMap,
    path::{Component, Path},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use futures_util::StreamExt;
use sqlx::{QueryBuilder, Row};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    sync::{Notify, Semaphore, mpsc, watch},
};

use crate::{
    proxy,
    runtime::{
        BuildSpec, ContainerSpec, ContainerState, LogMsg, LogSink, PlanSpec, Started, VolumeMount,
    },
    state::{AppState, new_id, now},
};

pub const LABEL_APP: &str = "rig.app";
/// Dockerfile path an app starts with. When it is missing, Railpack builds.
pub const DEFAULT_DOCKERFILE: &str = "Dockerfile";
pub const LABEL_DEPLOYMENT: &str = "rig.deployment";
pub const LABEL_MANAGED: &str = "rig.managed";

const DEPLOY_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const CLONE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const HEALTH_TIMEOUT: Duration = Duration::from_secs(90);
/// How long the old container gets to finish in-flight requests.
const STOP_GRACE_SECS: u32 = 10;
/// Maximum concurrent deployments across different apps. Source builds still
/// run one at a time via `build_lock` to protect RAM on small VPSes.
const MAX_CONCURRENT_DEPLOYS: usize = 4;
/// Number of most recent finished deployments per app whose raw build logs are kept.
const RETAIN_LOG_DEPLOYMENTS_PER_APP: i64 = 30;

/// Wakes the worker and per-deployment log followers.
pub struct Deployer {
    queued: Notify,
    /// Per-deployment watch channels bumped whenever log lines or status change.
    activity: std::sync::Mutex<HashMap<String, watch::Sender<u64>>>,
    http: reqwest::Client,
    /// Active deployment cancellation handles keyed by deployment id.
    cancels: std::sync::Mutex<HashMap<String, watch::Sender<bool>>>,
    /// Signalled when the server is shutting down so in-flight swaps can clean
    /// up and restore any paused stateful containers before process exit.
    shutdown: watch::Sender<bool>,
    /// Permits for concurrent deployments across different apps.
    slots: Arc<Semaphore>,
    /// Single-permit lock protecting heavy Git/Railpack source builds.
    build_lock: Arc<Semaphore>,
}

impl Deployer {
    pub fn new() -> Result<Self> {
        Ok(Self {
            queued: Notify::new(),
            activity: std::sync::Mutex::new(HashMap::new()),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(3))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .context("building health check client")?,
            cancels: std::sync::Mutex::new(HashMap::new()),
            shutdown: watch::channel(false).0,
            slots: Arc::new(Semaphore::new(MAX_CONCURRENT_DEPLOYS)),
            build_lock: Arc::new(Semaphore::new(1)),
        })
    }

    /// Signal the deployment worker to stop starting new deployments, abort
    /// any in-flight build/health-check, and restore paused stateful containers.
    pub async fn shutdown(&self) {
        let _ = self.shutdown.send(true);
        self.queued.notify_waiters();
        let _ = tokio::time::timeout(
            Duration::from_secs(15),
            self.slots.acquire_many(MAX_CONCURRENT_DEPLOYS as u32),
        )
        .await;
    }

    pub fn is_shutting_down(&self) -> bool {
        *self.shutdown.borrow()
    }

    pub fn subscribe_shutdown(&self) -> watch::Receiver<bool> {
        self.shutdown.subscribe()
    }

    /// A receiver that changes whenever `deployment_id`'s state or logs change.
    pub fn subscribe_deployment(&self, deployment_id: &str) -> watch::Receiver<u64> {
        let mut map = self.activity.lock().unwrap_or_else(|p| p.into_inner());
        map.retain(|_, tx| tx.receiver_count() > 0);
        map.entry(deployment_id.to_string())
            .or_insert_with(|| watch::channel(0).0)
            .subscribe()
    }

    pub fn touch_deployment(&self, deployment_id: &str) {
        let mut map = self.activity.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(tx) = map.get(deployment_id) {
            tx.send_modify(|n| *n = n.wrapping_add(1));
            if tx.receiver_count() == 0 {
                map.remove(deployment_id);
            }
        }
    }

    fn register_cancel(&self, deployment_id: &str) -> watch::Receiver<bool> {
        let (tx, rx) = watch::channel(false);
        self.cancels
            .lock()
            .expect("cancel lock")
            .insert(deployment_id.to_string(), tx);
        rx
    }

    fn unregister_cancel(&self, deployment_id: &str) {
        self.cancels.lock().expect("cancel lock").remove(deployment_id);
    }

    /// Signals a currently running deployment to cancel. Returns `true` if a
    /// live worker handle was signalled.
    pub fn signal_cancel(&self, deployment_id: &str) -> bool {
        if let Some(tx) = self.cancels.lock().expect("cancel lock").get(deployment_id) {
            let _ = tx.send(true);
            true
        } else {
            false
        }
    }
}

pub struct NewDeployment<'a> {
    pub org_id: &'a str,
    pub app_id: &'a str,
    pub trigger: &'a str,
    pub created_by: Option<&'a str>,
    /// Reuse this image instead of building or pulling the app's source.
    pub image_ref: Option<&'a str>,
    /// With `image_ref`: the exact image to run, when the engine still has it.
    pub image_id: Option<&'a str>,
    /// Optional staged local source directory for `rig up --remote-build`.
    pub source_dir: Option<&'a str>,
    /// Optional short label/message recorded on the deployment.
    pub commit_msg: Option<&'a str>,
}

/// Queue a deployment and wake the worker. Returns the deployment id.
/// Fails with a unique constraint error (mapped to 409 Conflict) if an active
/// deployment already exists for `d.app_id`.
pub async fn enqueue(state: &AppState, d: NewDeployment<'_>) -> Result<String, sqlx::Error> {
    let id = new_id();
    sqlx::query(
        "INSERT INTO deployments (id, org_id, app_id, status, trigger, image_ref, image_id, source_dir, commit_msg, created_by, created_at)
         VALUES (?, ?, ?, 'queued', ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(d.org_id)
    .bind(d.app_id)
    .bind(d.trigger)
    .bind(d.image_ref)
    .bind(d.image_id)
    .bind(d.source_dir)
    .bind(d.commit_msg)
    .bind(d.created_by)
    .bind(now())
    .execute(&state.db)
    .await?;
    state.deploys.queued.notify_one();
    state.deploys.touch_deployment(&id);
    Ok(id)
}

/// Whether `e` is a SQLite unique-constraint violation from the active-deployment index.
pub fn is_active_deploy_conflict(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::Database(db) if db.is_unique_violation())
}

/// Recover deployments that were interrupted by a server restart or crash,
/// removing any half-started containers and resuming any stateful app
/// containers that were paused for a volume swap.
pub async fn recover(state: &AppState) {
    let interrupted_rows = sqlx::query(
        "SELECT id, app_id FROM deployments WHERE status IN ('building', 'deploying')",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    for r in &interrupted_rows {
        let dep_id: String = r.get("id");
        if let Ok(cids) = state.runtime.list_by_label(LABEL_DEPLOYMENT, &dep_id).await {
            for cid in cids {
                let _ = state.runtime.remove(&cid).await;
            }
        }
    }

    let interrupted = sqlx::query(
        "UPDATE deployments SET status = 'failed', finished_at = ?,
                error = 'Rig restarted while this deployment was running. Deploy again.'
          WHERE status IN ('building', 'deploying')",
    )
    .bind(now())
    .execute(&state.db)
    .await;
    if let Err(e) = interrupted {
        tracing::error!(error = %e, "could not reset interrupted deployments");
    }
    let _ = sqlx::query(
        "UPDATE apps SET status = CASE WHEN current_deployment_id IS NULL THEN 'failed' ELSE 'running' END
          WHERE status = 'deploying'
            AND id NOT IN (SELECT app_id FROM deployments WHERE status = 'queued')",
    )
    .execute(&state.db)
    .await;

    // Ensure every running app's live container is actually started (a stateful
    // app stopped mid-deploy before a hard kill would otherwise stay exited)
    // and its recorded loopback port matches the engine.
    let live_apps = sqlx::query(
        "SELECT a.id AS app_id, d.id AS deployment_id, d.container_id, d.host_port
           FROM apps a JOIN deployments d ON d.id = a.current_deployment_id
          WHERE a.status = 'running' AND d.container_id IS NOT NULL",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    for r in live_apps {
        let dep_id: String = r.get("deployment_id");
        let cid: String = r.get("container_id");
        let stored_port: Option<i64> = r.get("host_port");
        if let Ok(Some(health)) = state.runtime.health(&cid).await {
            if matches!(health.state, ContainerState::Exited(_) | ContainerState::Other(_)) {
                let _ = state.runtime.start(&cid).await;
            }
            if let Ok(Some(after)) = state.runtime.health(&cid).await
                && let Some(live_port) = after.host_port
                && stored_port != Some(i64::from(live_port))
            {
                let _ = sqlx::query("UPDATE deployments SET host_port = ? WHERE id = ?")
                    .bind(i64::from(live_port))
                    .bind(&dep_id)
                    .execute(&state.db)
                    .await;
            }
        }
    }
}

/// Run the deployment worker until the process exits.
///
/// Up to [`MAX_CONCURRENT_DEPLOYS`] deployments across different apps may run
/// simultaneously (e.g. rollbacks and prebuilt image deploys proceed without
/// waiting behind another app's source build), while Git/Railpack builds
/// remain serialized by `build_lock` to protect RAM on small VPSes.
pub async fn worker(state: AppState) {
    recover(&state).await;
    if let Err(e) = proxy::sync(&state).await {
        tracing::warn!(error = %format!("{e:#}"), "proxy not updated at startup");
    }

    let mut shutdown_rx = state.deploys.subscribe_shutdown();
    loop {
        if state.deploys.is_shutting_down() {
            break;
        }
        let permit = tokio::select! {
            p = state.deploys.slots.clone().acquire_owned() => match p {
                Ok(p) => p,
                Err(_) => break,
            },
            _ = shutdown_rx.wait_for(|&s| s) => break,
        };
        if state.deploys.is_shutting_down() {
            drop(permit);
            break;
        }
        let next = sqlx::query_scalar::<_, String>(
            "SELECT id FROM deployments WHERE status = 'queued' ORDER BY created_at, id LIMIT 1",
        )
        .fetch_optional(&state.db)
        .await;
        match next {
            Ok(Some(id)) => {
                // Atomically claim the queued row so the loop can immediately
                // look for another app's queued deployment without double-spawning.
                let claimed = sqlx::query(
                    "UPDATE deployments SET status = 'building', started_at = COALESCE(started_at, ?)
                      WHERE id = ? AND status = 'queued'",
                )
                .bind(now())
                .bind(&id)
                .execute(&state.db)
                .await;
                match claimed {
                    Ok(res) if res.rows_affected() == 1 => {
                        state.deploys.touch_deployment(&id);
                        let st = state.clone();
                        tokio::spawn(async move {
                            let _permit = permit;
                            run(&st, &id).await;
                        });
                    }
                    Ok(_) => drop(permit),
                    Err(e) => {
                        drop(permit);
                        tracing::error!(error = %e, "claiming queued deployment");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
            Ok(None) => {
                drop(permit);
                tokio::select! {
                    _ = state.deploys.queued.notified() => {}
                    _ = shutdown_rx.wait_for(|&s| s) => break,
                }
            }
            Err(e) => {
                drop(permit);
                tracing::error!(error = %e, "reading the deployment queue");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

struct AppRow {
    id: String,
    slug: String,
    source_kind: String,
    image: Option<String>,
    git_url: Option<String>,
    git_branch: Option<String>,
    dockerfile_path: String,
    port: u16,
    memory_mb: Option<u32>,
    cpu_millis: Option<u32>,
    health_path: Option<String>,
    current_deployment_id: Option<String>,
    project_id: String,
    org_id: String,
    command: Option<Vec<String>>,
    entrypoint: Option<Vec<String>>,
    extra_networks: Vec<String>,
}

/// An image or staged source fixed when the deployment was queued.
struct Preset {
    image_ref: Option<String>,
    image_id: Option<String>,
    source_dir: Option<String>,
}

fn parse_json_vec(raw: Option<String>) -> Option<Vec<String>> {
    let s = raw?;
    serde_json::from_str::<Vec<String>>(&s)
        .ok()
        .filter(|v| !v.is_empty())
}

async fn load_app(state: &AppState, deployment_id: &str) -> Result<(AppRow, Preset)> {
    let r = sqlx::query(
        "SELECT a.id, a.slug, a.source_kind, a.image, a.git_url, a.git_branch, a.dockerfile_path,
                a.port, a.memory_mb, a.cpu_millis, a.health_path, a.current_deployment_id,
                a.project_id, a.org_id, a.command, a.entrypoint, a.extra_networks,
                d.image_ref, d.image_id, d.source_dir
           FROM deployments d JOIN apps a ON a.id = d.app_id
          WHERE d.id = ?",
    )
    .bind(deployment_id)
    .fetch_one(&state.db)
    .await
    .context("loading the app")?;
    let app = AppRow {
        id: r.get("id"),
        slug: r.get("slug"),
        source_kind: r.get("source_kind"),
        image: r.get("image"),
        git_url: r.get("git_url"),
        git_branch: r.get("git_branch"),
        dockerfile_path: r.get("dockerfile_path"),
        port: u16::try_from(r.get::<i64, _>("port")).context("app port out of range")?,
        memory_mb: r
            .get::<Option<i64>, _>("memory_mb")
            .and_then(|v| u32::try_from(v).ok()),
        cpu_millis: r
            .get::<Option<i64>, _>("cpu_millis")
            .and_then(|v| u32::try_from(v).ok()),
        health_path: r.get("health_path"),
        current_deployment_id: r.get("current_deployment_id"),
        project_id: r.get("project_id"),
        org_id: r.get("org_id"),
        command: parse_json_vec(r.get("command")),
        entrypoint: parse_json_vec(r.get("entrypoint")),
        extra_networks: parse_json_vec(r.get("extra_networks")).unwrap_or_default(),
    };
    let preset = Preset {
        image_ref: r.get("image_ref"),
        image_id: r.get("image_id"),
        source_dir: r.get("source_dir"),
    };
    Ok((app, preset))
}

async fn run(state: &AppState, deployment_id: &str) {
    if state.deploys.is_shutting_down() {
        return;
    }
    let (tx, rx) = mpsc::channel::<LogMsg>(1024);
    let log = LogSink::new(tx);
    let writer = tokio::spawn(write_logs(state.clone(), deployment_id.to_string(), rx));
    let mut cancel_rx = state.deploys.register_cancel(deployment_id);
    let mut shutdown_rx = state.deploys.subscribe_shutdown();

    let cancelled;
    let shutting_down;
    let outcome = tokio::select! {
        res = tokio::time::timeout(DEPLOY_TIMEOUT, pipeline(state, deployment_id, &log)) => {
            cancelled = false;
            shutting_down = false;
            match res {
                Ok(r) => r,
                Err(_) => Err(anyhow!(
                    "the deployment took longer than 30 minutes and was stopped"
                )),
            }
        }
        _ = cancel_rx.wait_for(|&c| c) => {
            cancelled = true;
            shutting_down = false;
            Ok(())
        }
        _ = shutdown_rx.wait_for(|&s| s) => {
            cancelled = false;
            shutting_down = true;
            Err(anyhow!("Rig restarted while this deployment was running. Deploy again."))
        }
    };
    state.deploys.unregister_cancel(deployment_id);

    if cancelled || shutting_down || outcome.is_err() {
        cleanup_aborted_deployment(state, deployment_id, &log).await;
    }

    if cancelled {
        log.line("deploy", "Deployment cancelled").await;
        let _ = sqlx::query(
            "UPDATE deployments SET status = 'cancelled', error = 'Cancelled by user', finished_at = ?
              WHERE id = ? AND status IN ('queued', 'building', 'deploying')",
        )
        .bind(now())
        .bind(deployment_id)
        .execute(&state.db)
        .await;
        let _ = sqlx::query(
            "UPDATE apps SET updated_at = ?,
                    status = CASE WHEN current_deployment_id IS NULL THEN 'idle' ELSE 'running' END
              WHERE id = (SELECT app_id FROM deployments WHERE id = ?) AND status = 'deploying'",
        )
        .bind(now())
        .bind(deployment_id)
        .execute(&state.db)
        .await;
    } else if let Err(e) = &outcome {
        // The alternate form includes the cause chain, which is what tells
        // the user whether the image, the build or the container failed.
        let message = format!("{e:#}");
        log.line("deploy", format!("Deployment failed: {message}"))
            .await;
        let res = sqlx::query(
            "UPDATE deployments SET status = 'failed', error = ?, finished_at = ? WHERE id = ?",
        )
        .bind(&message)
        .bind(now())
        .bind(deployment_id)
        .execute(&state.db)
        .await;
        if let Err(e) = res {
            tracing::error!(error = %e, "recording a failed deployment");
        }
        let app: Option<(String, String, String)> = sqlx::query_as(
            "SELECT a.org_id, a.id, a.name FROM apps a JOIN deployments d ON d.app_id = a.id
              WHERE d.id = ?",
        )
        .bind(deployment_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
        if let Some((org_id, app_id, name)) = app {
            crate::alerts::notify(
                state,
                &org_id,
                crate::alerts::Alert {
                    event: rig_api::AlertEvent::DeployFailed,
                    title: format!("Deploying {name} failed"),
                    detail: message.clone(),
                    target: Some(app_id),
                },
            );
        }
        let _ = sqlx::query(
            "UPDATE apps SET updated_at = ?,
                    status = CASE WHEN current_deployment_id IS NULL THEN 'failed' ELSE 'running' END
              WHERE id = (SELECT app_id FROM deployments WHERE id = ?) AND status = 'deploying'",
        )
        .bind(now())
        .bind(deployment_id)
        .execute(&state.db)
        .await;
    }

    drop(log);
    let _ = writer.await;
    prune_old_deployment_logs(&state.db, deployment_id).await;
    state.deploys.touch_deployment(deployment_id);
}

/// Keeps raw build/deploy log lines bounded to the most recent
/// [`RETAIN_LOG_DEPLOYMENTS_PER_APP`] finished deployments per app while
/// preserving all deployment metadata rows.
async fn prune_old_deployment_logs(db: &sqlx::SqlitePool, deployment_id: &str) {
    let app_id: Option<String> =
        sqlx::query_scalar("SELECT app_id FROM deployments WHERE id = ?")
            .bind(deployment_id)
            .fetch_optional(db)
            .await
            .ok()
            .flatten();
    let Some(app_id) = app_id else {
        return;
    };
    if let Err(e) = sqlx::query(
        "DELETE FROM deployment_logs
          WHERE deployment_id IN (
            SELECT id FROM deployments
             WHERE app_id = ?1
               AND status NOT IN ('queued', 'building', 'deploying')
               AND id != COALESCE((SELECT current_deployment_id FROM apps WHERE id = ?1), '')
             ORDER BY created_at DESC, id DESC
             LIMIT -1 OFFSET ?2
          )",
    )
    .bind(&app_id)
    .bind(RETAIN_LOG_DEPLOYMENTS_PER_APP)
    .execute(db)
    .await
    {
        tracing::warn!(error = %e, app = %app_id, "pruning old deployment logs");
    }
}

async fn set_status(state: &AppState, deployment_id: &str, status: &str) -> Result<()> {
    sqlx::query(
        "UPDATE deployments SET status = ?, started_at = COALESCE(started_at, ?) WHERE id = ?",
    )
    .bind(status)
    .bind(now())
    .bind(deployment_id)
    .execute(&state.db)
    .await?;
    state.deploys.touch_deployment(deployment_id);
    Ok(())
}

/// Memory limit for apps that do not set one.
pub const DEFAULT_APP_MEMORY_MB: u32 = 512;

/// Builds and pulls stop below this much free space on the data disk, before
/// a full disk can corrupt the database or the running apps.
pub const MIN_FREE_DISK: u64 = 1024 * 1024 * 1024;

/// Free bytes when they are below [`MIN_FREE_DISK`]. An unknown total
/// (`0`, for example on an unsupported filesystem) never blocks.
pub fn short_of_disk(total: u64, used: u64) -> Option<u64> {
    let free = total.saturating_sub(used);
    (total > 0 && free < MIN_FREE_DISK).then_some(free)
}

/// Fail early, with a useful message, when the data disk is nearly full.
pub async fn ensure_disk_space(state: &AppState) -> Result<()> {
    ensure_disk_space_with_engine(state, None).await
}

async fn ensure_disk_space_with_engine(state: &AppState, engine_name: Option<&str>) -> Result<()> {
    let dir = state.config.data_dir.clone();
    let (disk_used, disk_total) =
        tokio::task::spawn_blocking(move || crate::metrics::sample_disk(&dir)).await?;
    if let Some(free) = short_of_disk(disk_total, disk_used) {
        let engine = match engine_name {
            Some(name) => name.to_owned(),
            None => state
                .runtime
                .info()
                .await
                .map(|e| e.name)
                .unwrap_or_else(|_| "docker".into()),
        };
        bail!(
            "only {} MB free on the disk that holds {}. Remove old backups, or run \
             `{engine} system prune` if the engine stores its images on the same disk, \
             then try again",
            free / (1024 * 1024),
            state.config.data_dir.display()
        );
    }
    Ok(())
}

async fn pipeline(state: &AppState, deployment_id: &str, log: &LogSink) -> Result<()> {
    let (app, preset) = load_app(state, deployment_id).await?;
    let short = &deployment_id[deployment_id.len().saturating_sub(8)..];

    set_status(state, deployment_id, "building").await?;
    sqlx::query("UPDATE apps SET status = 'deploying', updated_at = ? WHERE id = ?")
        .bind(now())
        .bind(&app.id)
        .execute(&state.db)
        .await?;

    let engine = state.runtime.info().await.context(
        "the container engine is not reachable. Check that Docker or Podman is running and that Rig can use its socket",
    )?;
    log.line(
        "deploy",
        format!("Using {} {}", engine.name, engine.version),
    )
    .await;
    ensure_disk_space_with_engine(state, Some(&engine.name)).await?;

    // ---- image
    // A rollback runs the exact image it ran before, if the engine still has it.
    let pinned = match &preset.image_id {
        Some(id) => state.runtime.image_id(id).await?,
        None => None,
    };
    let image = if let Some(source_dir) = preset.source_dir.as_deref() {
        let _build_permit = match state.deploys.build_lock.try_acquire() {
            Ok(p) => p,
            Err(_) => {
                log.line("build", "Waiting for another build to finish").await;
                state
                    .deploys
                    .build_lock
                    .acquire()
                    .await
                    .context("build queue closed")?
            }
        };
        let tag = format!("rig/{}:{short}", app.slug);
        let res = build_from_local_dir(state, &app, Path::new(source_dir), &tag, log).await;
        let _ = tokio::fs::remove_dir_all(source_dir).await;
        res?;
        tag
    } else {
        match (preset.image_ref, app.source_kind.as_str()) {
            (Some(image), _) if pinned.is_some() => {
                log.line("build", format!("Reusing image {image}")).await;
                image
            }
            (Some(image), "git" | "local") => {
                log.line("build", format!("Reusing image {image}")).await;
                image
            }
            (Some(image), _) => {
                log.line("build", format!("Pulling {image}")).await;
                state.runtime.pull(&image, log).await?;
                image
            }
            (None, "git") => {
                let _build_permit = match state.deploys.build_lock.try_acquire() {
                    Ok(p) => p,
                    Err(_) => {
                        log.line("build", "Waiting for another build to finish").await;
                        state
                            .deploys
                            .build_lock
                            .acquire()
                            .await
                            .context("build queue closed")?
                    }
                };
                let tag = format!("rig/{}:{short}", app.slug);
                build_from_git(state, deployment_id, &app, &tag, log).await?;
                tag
            }
            (None, "local") => {
                bail!("local apps are deployed from your workstation with `rig up`");
            }
            (None, _) => {
                let image = app.image.clone().context("the app has no image set")?;
                log.line("build", format!("Pulling {image}")).await;
                state.runtime.pull(&image, log).await?;
                image
            }
        }
    };
    let image_id = match pinned {
        Some(id) => id,
        None => state.runtime.image_id(&image).await?.with_context(|| {
            format!("the image {image} is not available after pulling or building it")
        })?,
    };
    sqlx::query("UPDATE deployments SET image_ref = ?, image_id = ? WHERE id = ?")
        .bind(&image)
        .bind(&image_id)
        .bind(deployment_id)
        .execute(&state.db)
        .await?;

    // ---- container
    set_status(state, deployment_id, "deploying").await?;
    let mut env = load_env(state, &app.id).await?;
    if !env.iter().any(|(k, _)| k == "PORT") {
        env.push(("PORT".into(), app.port.to_string()));
    }
    let volumes = app_volumes(state, &app.id).await?;
    let spec = ContainerSpec {
        name: format!("rig-{}-{short}", app.slug),
        image: image_id.clone(),
        port: app.port,
        env,
        labels: HashMap::from([
            (LABEL_MANAGED.to_string(), "true".to_string()),
            (LABEL_APP.to_string(), app.id.clone()),
            (LABEL_DEPLOYMENT.to_string(), deployment_id.to_string()),
        ]),
        // Small servers must survive one leaky app, so nothing runs unbounded.
        memory_mb: Some(app.memory_mb.unwrap_or(DEFAULT_APP_MEMORY_MB)),
        cpu_millis: app.cpu_millis,
        command: app.command.clone(),
        entrypoint: app.entrypoint.clone(),
        // Shared with the project's databases, so apps reach them by name.
        network: Some(project_network(&app.project_id)),
        aliases: vec![app.slug.clone()],
        extra_networks: app.extra_networks.clone(),
        volumes: volumes.clone(),
    };

    // Two containers must never write one volume, so stateful apps with writable
    // mounts stop the old container first and accept a few seconds of downtime.
    let has_writable_volumes = volumes.iter().any(|v| !v.read_only);
    let mut paused = Vec::new();
    if has_writable_volumes {
        for id in state.runtime.list_by_label(LABEL_APP, &app.id).await? {
            log.line(
                "deploy",
                "Stopping the running container: this app has writable volumes",
            )
            .await;
            state.runtime.stop(&id, STOP_GRACE_SECS).await?;
            paused.push(id);
        }
    }

    log.line("deploy", format!("Starting container {}", spec.name))
        .await;
    let started = match state.runtime.run(&spec).await {
        Ok(started) => started,
        Err(e) => {
            resume(state, &paused, log).await;
            return Err(e);
        }
    };

    if let Err(e) = wait_healthy(state, &started, &app, log).await {
        tail_container(state, &started.id, log).await;
        let _ = state.runtime.remove(&started.id).await;
        resume(state, &paused, log).await;
        return Err(e);
    }

    // ---- switch
    let previous = app
        .current_deployment_id
        .as_deref()
        .filter(|p| *p != deployment_id);
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "UPDATE deployments SET status = 'running', container_id = ?, host_port = ?, finished_at = ?
          WHERE id = ?",
    )
    .bind(&started.id)
    .bind(i64::from(started.host_port))
    .bind(now())
    .bind(deployment_id)
    .execute(&mut *tx)
    .await?;
    if let Some(prev) = previous {
        sqlx::query(
            "UPDATE deployments SET status = 'superseded' WHERE id = ? AND status = 'running'",
        )
        .bind(prev)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "UPDATE apps SET status = 'running', current_deployment_id = ?, restart_count = 0,
                     last_crash_at = NULL, last_crash_reason = NULL, updated_at = ? WHERE id = ?",
    )
    .bind(deployment_id)
    .bind(now())
    .bind(&app.id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    match proxy::sync(state).await {
        Ok(()) => log.line("deploy", "Proxy routes updated").await,
        Err(e) => {
            log.line(
                "deploy",
                format!(
                    "Warning: the proxy was not updated ({e:#}). The app is running on 127.0.0.1:{} \
                     and its domains will work once the proxy is reachable.",
                    started.host_port
                ),
            )
            .await;
        }
    }

    // ---- retire everything else that belongs to this app
    for id in state
        .runtime
        .list_by_label(LABEL_APP, &app.id)
        .await
        .unwrap_or_default()
    {
        if id == started.id {
            continue;
        }
        if let Err(e) = state.runtime.stop(&id, STOP_GRACE_SECS).await {
            tracing::warn!(error = %format!("{e:#}"), "stopping an old container");
        }
        if let Err(e) = state.runtime.remove(&id).await {
            tracing::warn!(error = %format!("{e:#}"), "removing an old container");
        }
    }
    log.line("deploy", format!("Live on 127.0.0.1:{}", started.host_port))
        .await;
    Ok(())
}

/// Restart containers stopped for a stateful swap after the new one failed.
async fn resume(state: &AppState, ids: &[String], log: &LogSink) {
    for id in ids {
        match state.runtime.start(id).await {
            Ok(()) => log.line("deploy", "Restarted the previous container").await,
            Err(e) => {
                log.line(
                    "deploy",
                    format!("Could not restart the previous container: {e:#}"),
                )
                .await;
            }
        }
    }
}

/// Remove any half-started container from an aborted/failed deployment and
/// make sure the app's previous live container (if it was paused for a volume
/// swap) is running again.
async fn cleanup_aborted_deployment(state: &AppState, deployment_id: &str, log: &LogSink) {
    let is_live: Option<String> = sqlx::query_scalar(
        "SELECT id FROM apps WHERE current_deployment_id = ?",
    )
    .bind(deployment_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    if is_live.is_some() {
        return;
    }
    if let Ok(ids) = state
        .runtime
        .list_by_label(LABEL_DEPLOYMENT, deployment_id)
        .await
    {
        for id in ids {
            let _ = state.runtime.remove(&id).await;
        }
    }
    let prev_cid: Option<String> = sqlx::query_scalar(
        "SELECT prev.container_id
           FROM deployments cur
           JOIN apps a ON a.id = cur.app_id
           JOIN deployments prev ON prev.id = a.current_deployment_id
          WHERE cur.id = ? AND prev.container_id IS NOT NULL",
    )
    .bind(deployment_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    if let Some(cid) = prev_cid
        && let Ok(Some(h)) = state.runtime.health(&cid).await
        && !matches!(h.state, ContainerState::Running)
    {
        resume(state, std::slice::from_ref(&cid), log).await;
    }
}

/// Volume/bind mounts attached to an app.
async fn app_volumes(state: &AppState, app_id: &str) -> Result<Vec<VolumeMount>> {
    let rows = sqlx::query(
        "SELECT name, mount_path, read_only FROM app_volumes WHERE app_id = ? ORDER BY mount_path",
    )
    .bind(app_id)
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| VolumeMount {
            source: r.get("name"),
            target: r.get("mount_path"),
            read_only: r.get::<i64, _>("read_only") != 0,
        })
        .collect())
}

/// Delete an app's Rig-owned named volumes. Preserves adopted host bind mounts
/// and pre-existing external volumes (`owned = 0` or `kind = 'bind'`).
pub async fn remove_app_volumes(state: &AppState, app_id: &str) -> Result<()> {
    let owned: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM app_volumes WHERE app_id = ? AND kind = 'volume' AND owned = 1",
    )
    .bind(app_id)
    .fetch_all(&state.db)
    .await?;
    for name in owned {
        state.runtime.remove_volume(&name).await?;
    }
    Ok(())
}

async fn load_env(state: &AppState, app_id: &str) -> Result<Vec<(String, String)>> {
    let rows = sqlx::query("SELECT key, value_enc FROM env_vars WHERE app_id = ? ORDER BY key")
        .bind(app_id)
        .fetch_all(&state.db)
        .await?;
    rows.iter()
        .map(|r| {
            let key: String = r.get("key");
            let value = state
                .vault
                .decrypt(&r.get::<String, _>("value_enc"))
                .with_context(|| format!("decrypting environment variable {key}"))?;
            Ok((key, value))
        })
        .collect()
}

/// Wait until a freshly started container passes health checks (2 consecutive
/// passes on HTTP `health_path` or TCP port). Shared by `deploy` and `adopt`.
pub async fn wait_container_healthy(
    state: &AppState,
    started: &Started,
    port: u16,
    health_path: Option<&str>,
    log: &LogSink,
) -> Result<()> {
    const BACKOFF_MS: &[u64] = &[100, 150, 250, 500, 700];
    let what = match health_path {
        Some(path) => format!("GET {path} on port {port}"),
        None => format!("port {port}"),
    };
    log.line("deploy", format!("Waiting for {what}")).await;
    let begin = Instant::now();
    // An engine's port forwarder can accept connections before the process
    // listens, so one success is not enough: the container must also still
    // be running on the next look.
    let mut passes = 0;
    let mut attempt: usize = 0;
    loop {
        match state.runtime.state(&started.id).await? {
            ContainerState::Exited(code) => bail!(
                "the container exited{} before it became healthy. Its last output is above",
                code.map(|c| format!(" with code {c}")).unwrap_or_default()
            ),
            ContainerState::Missing => bail!("the container disappeared before it became healthy"),
            ContainerState::Running | ContainerState::Other(_) => {}
        }
        let ok = match health_path {
            Some(path) => {
                let url = format!("http://127.0.0.1:{}{path}", started.host_port);
                matches!(state.deploys.http.get(&url).send().await, Ok(r) if r.status().as_u16() < 400)
            }
            None => tcp_ready(started.host_port).await,
        };
        passes = if ok { passes + 1 } else { 0 };
        if passes >= 2 {
            log.line(
                "deploy",
                format!("Healthy after {:.1}s", begin.elapsed().as_secs_f32()),
            )
            .await;
            return Ok(());
        }
        if begin.elapsed() > HEALTH_TIMEOUT {
            bail!(
                "nothing answered on {what} within {}s. Check that the app listens on 0.0.0.0:{port} \
                 (not 127.0.0.1) and that the port setting matches",
                HEALTH_TIMEOUT.as_secs(),
            );
        }
        let delay_ms = BACKOFF_MS[attempt.min(BACKOFF_MS.len() - 1)];
        attempt += 1;
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }
}

/// Wait until the new container answers, or explain why it does not.
async fn wait_healthy(
    state: &AppState,
    started: &Started,
    app: &AppRow,
    log: &LogSink,
) -> Result<()> {
    wait_container_healthy(state, started, app.port, app.health_path.as_deref(), log).await
}

/// The engine network shared by one project's apps and databases. Projects
/// cannot reach each other's containers.
pub fn project_network(project_id: &str) -> String {
    let tail = &project_id[project_id.len().saturating_sub(12)..];
    format!("rig-{tail}")
}

/// Whether something really listens behind a published loopback port.
///
/// Docker's port forwarder accepts a connection even when nothing listens in
/// the container yet, then closes it. So after connecting we wait briefly:
/// a close means "not yet", silence or data means a real server holds it.
pub async fn tcp_ready(host_port: u16) -> bool {
    use tokio::io::AsyncReadExt;
    let connect = tokio::net::TcpStream::connect(("127.0.0.1", host_port));
    let Ok(Ok(mut stream)) = tokio::time::timeout(Duration::from_secs(2), connect).await else {
        return false;
    };
    let mut byte = [0u8; 1];
    match tokio::time::timeout(Duration::from_millis(150), stream.read(&mut byte)).await {
        Err(_) => true,
        Ok(Ok(n)) => n > 0,
        Ok(Err(_)) => false,
    }
}

/// Copy the container's last output into the deployment log.
async fn tail_container(state: &AppState, id: &str, log: &LogSink) {
    let mut lines = state.runtime.logs(id, 40, false);
    while let Some(Ok(msg)) = lines.next().await {
        log.line(msg.stream, msg.line).await;
    }
}

// --------------------------------------------------------- local source build

/// Build an image from a staged local context uploaded by `rig up --remote-build`.
/// Supports `Dockerfile`, `Containerfile`, or Railpack auto-detection.
async fn build_from_local_dir(
    state: &AppState,
    app: &AppRow,
    context: &Path,
    tag: &str,
    log: &LogSink,
) -> Result<()> {
    validate_relative_path(&app.dockerfile_path).map_err(|m| anyhow!(m))?;
    let ctx_check = context.to_path_buf();
    tokio::task::spawn_blocking(move || verify_no_symlinks(&ctx_check))
        .await
        .context("verifying uploaded context safety")??;

    // Resolve Dockerfile or Containerfile when using default dockerfile_path.
    let candidates: Vec<&str> = if app.dockerfile_path == DEFAULT_DOCKERFILE {
        vec![DEFAULT_DOCKERFILE, "Containerfile"]
    } else {
        vec![app.dockerfile_path.as_str()]
    };
    let mut chosen_dockerfile = None;
    for candidate in candidates {
        let meta = tokio::fs::symlink_metadata(context.join(candidate)).await;
        match meta {
            Ok(m) if m.file_type().is_symlink() => {
                bail!("the {candidate} path cannot be a symbolic link");
            }
            Ok(m) if m.is_file() => {
                chosen_dockerfile = Some(candidate.to_string());
                break;
            }
            _ => {}
        }
    }

    let Some(dockerfile) = chosen_dockerfile else {
        if app.dockerfile_path != DEFAULT_DOCKERFILE {
            bail!(
                "no file at {} in the uploaded directory",
                app.dockerfile_path
            );
        }
        log.line(
            "build",
            "No Dockerfile or Containerfile found, building with Railpack",
        )
        .await;
        let builds = state.config.data_dir.join("builds");
        tokio::fs::create_dir_all(&builds).await?;
        let work = tempfile::Builder::new()
            .prefix("local-plan-")
            .tempdir_in(&builds)?;
        let bin = crate::railpack::binary(&state.config.data_dir, log).await?;
        let plan_dir = work.path().join("plan");
        crate::railpack::prepare(&bin, context, &plan_dir, log).await?;
        log.line("build", format!("Building {tag}")).await;
        return state
            .runtime
            .build_plan(
                PlanSpec {
                    context: context.to_path_buf(),
                    plan_dir,
                    work_dir: work.path().to_path_buf(),
                    frontend: crate::railpack::FRONTEND.to_string(),
                    tag: tag.to_string(),
                },
                log,
            )
            .await;
    };

    log.line(
        "build",
        format!("Building {tag} from uploaded context ({dockerfile})"),
    )
    .await;
    state
        .runtime
        .build(
            BuildSpec {
                context: context.to_path_buf(),
                dockerfile,
                tag: tag.to_string(),
                labels: HashMap::from([
                    (LABEL_MANAGED.to_string(), "true".to_string()),
                    (LABEL_APP.to_string(), app.id.clone()),
                ]),
            },
            log,
        )
        .await
}

// ------------------------------------------------------------------ git build

async fn build_from_git(
    state: &AppState,
    deployment_id: &str,
    app: &AppRow,
    tag: &str,
    log: &LogSink,
) -> Result<()> {
    let url = app
        .git_url
        .as_deref()
        .context("the app has no repository URL set")?;
    validate_git_url_with(url, state.config.allow_private_git).map_err(|m| anyhow!(m))?;
    validate_git_remote_ip(url, state.config.allow_private_git).await?;
    validate_relative_path(&app.dockerfile_path).map_err(|m| anyhow!(m))?;

    let builds = state.config.data_dir.join("builds");
    tokio::fs::create_dir_all(&builds)
        .await
        .context("creating the build directory")?;
    let dir = tempfile::Builder::new()
        .prefix("build-")
        .tempdir_in(&builds)
        .context("creating a build directory")?;
    let checkout = dir.path().join("src");

    log.line("build", format!("Cloning {url}")).await;
    let mut cmd = tokio::process::Command::new("git");
    // Strict protocol allowlist, no HTTP redirects (which could hop from a public
    // host to an internal address), and no filesystem symlinks (so host-side
    // Railpack cannot follow a symlink to host secrets).
    cmd.args(GIT_SAFE_CONFIG)
        .args(["clone", "--depth", "1", "--single-branch", "--no-tags"]);
    if let Some(branch) = app.git_branch.as_deref().filter(|b| !b.is_empty()) {
        cmd.args(["--branch", branch]);
    }
    cmd.arg("--").arg(url).arg(&checkout);
    // A repository the GitHub App is installed on is cloned with a short-lived
    // installation token. It travels in the environment (never the argument
    // list, the URL or .git/config) and is not logged.
    let auth = crate::github::clone_auth(state, &app.org_id, url).await;
    match &auth {
        Ok(Some(env)) => {
            log.line("build", "Using the GitHub App's access to this repository")
                .await;
            cmd.envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        }
        Ok(None) => {}
        Err(e) => {
            log.line(
                "build",
                format!("Could not get GitHub App access ({e:#}); trying without it"),
            )
            .await;
        }
    }
    run_logged(cmd, CLONE_TIMEOUT, log).await.context(
        "git clone failed. Check the repository URL and branch, and that the repository is \
         public or the GitHub App is installed on it",
    )?;

    // Defense in depth: verify the checked-out tree contains no symlinks
    // before reading any file or running host-side Railpack.
    let checkout_for_check = checkout.clone();
    tokio::task::spawn_blocking(move || verify_no_symlinks(&checkout_for_check))
        .await
        .context("verifying checkout safety")??;

    let sha = git_output(&checkout, &["rev-parse", "HEAD"])
        .await
        .unwrap_or_default();
    let msg = git_output(&checkout, &["log", "-1", "--pretty=%s"])
        .await
        .unwrap_or_default();
    if !sha.is_empty() {
        log.line(
            "build",
            format!("Commit {} {msg}", &sha[..sha.len().min(10)]),
        )
        .await;
        sqlx::query("UPDATE deployments SET commit_sha = ?, commit_msg = ? WHERE id = ?")
            .bind(&sha)
            .bind(msg.chars().take(200).collect::<String>())
            .bind(deployment_id)
            .execute(&state.db)
            .await?;
    }

    let dockerfile_meta = tokio::fs::symlink_metadata(checkout.join(&app.dockerfile_path)).await;
    let has_dockerfile = match dockerfile_meta {
        Ok(m) if m.file_type().is_symlink() => {
            bail!("the Dockerfile path cannot be a symbolic link");
        }
        Ok(m) => m.is_file(),
        Err(_) => false,
    };
    if !has_dockerfile {
        // With no Dockerfile at the default path, Railpack works out the
        // build from the source. A custom path that is missing is a mistake.
        if app.dockerfile_path != DEFAULT_DOCKERFILE {
            bail!(
                "no file at {} in the repository. Set the Dockerfile path in the app's settings",
                app.dockerfile_path
            );
        }
        log.line("build", "No Dockerfile found, building with Railpack")
            .await;
        let bin = crate::railpack::binary(&state.config.data_dir, log).await?;
        let plan_dir = dir.path().join("plan");
        crate::railpack::prepare(&bin, &checkout, &plan_dir, log).await?;
        log.line("build", format!("Building {tag}")).await;
        return state
            .runtime
            .build_plan(
                PlanSpec {
                    context: checkout,
                    plan_dir,
                    work_dir: dir.path().to_path_buf(),
                    frontend: crate::railpack::FRONTEND.to_string(),
                    tag: tag.to_string(),
                },
                log,
            )
            .await;
    }

    log.line("build", format!("Building {tag}")).await;
    state
        .runtime
        .build(
            BuildSpec {
                context: checkout,
                dockerfile: app.dockerfile_path.clone(),
                tag: tag.to_string(),
                labels: HashMap::from([
                    (LABEL_MANAGED.to_string(), "true".to_string()),
                    (LABEL_APP.to_string(), app.id.clone()),
                ]),
            },
            log,
        )
        .await
    // `dir` is removed here, on success and on failure.
}

/// Run a command, sending its output to the log. Fails on a non-zero exit.
async fn run_logged(
    mut cmd: tokio::process::Command,
    limit: Duration,
    log: &LogSink,
) -> Result<()> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "true")
        .kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .context("starting git. Is git installed on this server?")?;
    let out = child.stdout.take().expect("stdout is piped");
    let err = child.stderr.take().expect("stderr is piped");
    let pump = |reader: Box<dyn tokio::io::AsyncRead + Send + Unpin>, log: LogSink| async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            log.line("build", line).await;
        }
    };
    let work = async {
        let (_, _, status) = tokio::join!(
            pump(Box::new(out), log.clone()),
            pump(Box::new(err), log.clone()),
            child.wait()
        );
        status
    };
    let status = tokio::time::timeout(limit, work)
        .await
        .map_err(|_| anyhow!("timed out after {}s", limit.as_secs()))??;
    if !status.success() {
        bail!("exited with {status}");
    }
    Ok(())
}

async fn git_output(dir: &Path, args: &[&str]) -> Option<String> {
    let out = tokio::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .await
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Strict git configuration flags shared by `git clone` and `git ls-remote`.
pub const GIT_SAFE_CONFIG: &[&str] = &[
    "-c",
    "protocol.allow=never",
    "-c",
    "protocol.https.allow=always",
    "-c",
    "protocol.ssh.allow=always",
    "-c",
    "http.followRedirects=false",
    "-c",
    "core.symlinks=false",
];

/// Extract `(host, port)` from an `https://`, `ssh://`, or `git@host:path` URL.
pub fn git_url_host_port(url: &str) -> Option<(String, u16)> {
    if url.starts_with("https://") || url.starts_with("ssh://") {
        let parsed = url::Url::parse(url).ok()?;
        let host = parsed.host_str()?.to_owned();
        let default_port = if parsed.scheme() == "ssh" { 22 } else { 443 };
        return Some((host, parsed.port().unwrap_or(default_port)));
    }
    if let Some((user_host, _path)) = url.split_once(':')
        && let Some((_user, host)) = user_host.split_once('@')
        && !host.is_empty()
    {
        return Some((host.to_owned(), 22));
    }
    None
}

/// Resolve the Git host and refuse loopback, private, link-local, or CGNAT IPs
/// unless `allow_private` is enabled.
pub async fn validate_git_remote_ip(url: &str, allow_private: bool) -> Result<()> {
    let Some((host, port)) = git_url_host_port(url) else {
        bail!("the repository URL has no valid host");
    };
    crate::access::resolve_and_validate_host(&host, port, allow_private)
        .await
        .context("checking the repository host address")?;
    Ok(())
}

/// Reject any symbolic links anywhere inside the checked-out worktree (ignoring `.git`).
fn verify_no_symlinks(root: &Path) -> Result<()> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)
            .with_context(|| format!("inspecting checkout {}", dir.display()))?
        {
            let entry = entry?;
            let name = entry.file_name();
            if dir == root && name == ".git" {
                continue;
            }
            let ft = entry.file_type()?;
            if ft.is_symlink() {
                bail!("repository checkout contains a symbolic link, which is not allowed");
            }
            if ft.is_dir() {
                stack.push(entry.path());
            }
        }
    }
    Ok(())
}

/// Accept only remote repositories over HTTPS or SSH, blocking loopback/private
/// hosts by default.
#[cfg(test)]
pub fn validate_git_url(url: &str) -> Result<(), &'static str> {
    validate_git_url_with(url, false)
}

/// Accept only remote repositories over HTTPS or SSH.
///
/// Local paths, `file`/`ext` transports, and (unless `allow_private` is true)
/// internal/loopback hosts are rejected up front.
pub fn validate_git_url_with(url: &str, allow_private: bool) -> Result<(), &'static str> {
    if url.len() > 512 || url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("the repository URL contains spaces or control characters");
    }
    if url.starts_with('-') {
        return Err("the repository URL cannot start with a dash");
    }
    if let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("ssh://"))
    {
        if rest.is_empty() || rest.starts_with('/') || rest.starts_with('-') {
            return Err("the repository URL has no host");
        }
        let Ok(parsed) = url::Url::parse(url) else {
            return Err("the repository URL is not valid");
        };
        let Some(host) = parsed.host_str().filter(|h| !h.is_empty()) else {
            return Err("the repository URL has no host");
        };
        if !allow_private && crate::access::is_blocked_hostname(host) {
            return Err("the repository URL cannot point to a loopback, private, or local address");
        }
        return Ok(());
    }
    // scp-style: git@host:owner/repo.git
    if let Some((user_host, path)) = url.split_once(':')
        && let Some((user, host)) = user_host.split_once('@')
        && !user.is_empty()
        && !host.is_empty()
        && !path.is_empty()
        && !path.starts_with('/')
        && user
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
    {
        if !allow_private && crate::access::is_blocked_hostname(host) {
            return Err("the repository URL cannot point to a loopback, private, or local address");
        }
        return Ok(());
    }
    Err("use an https:// or ssh:// repository URL, or the git@host:owner/repo form")
}

/// A path inside the repository: relative, with no `..` and no root.
pub fn validate_relative_path(path: &str) -> Result<(), &'static str> {
    if path.is_empty() || path.len() > 255 || path.chars().any(char::is_control) {
        return Err("the path is empty, too long, or contains control characters");
    }
    if Path::new(path)
        .components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
    {
        Ok(())
    } else {
        Err("the path must stay inside the repository (no leading / and no ..)")
    }
}

// ----------------------------------------------------------------------- logs

/// Persist log lines in batched multi-row inserts so a chatty build costs few
/// transactions and minimal statement overhead.
async fn write_logs(state: AppState, deployment_id: String, mut rx: mpsc::Receiver<LogMsg>) {
    let mut seq: i64 = 0;
    let mut batch = Vec::with_capacity(128);
    while let Some(first) = rx.recv().await {
        batch.push(first);
        while batch.len() < 256 {
            match rx.try_recv() {
                Ok(m) => batch.push(m),
                Err(_) => break,
            }
        }
        let ts = now();
        let result: Result<(), sqlx::Error> = async {
            let mut tx = state.db.begin().await?;
            for chunk in batch.chunks(100) {
                let mut qb: QueryBuilder<sqlx::Sqlite> = QueryBuilder::new(
                    "INSERT INTO deployment_logs (deployment_id, seq, ts, stream, line) ",
                );
                qb.push_values(chunk, |mut b, m| {
                    seq += 1;
                    b.push_bind(&deployment_id)
                        .push_bind(seq)
                        .push_bind(&ts)
                        .push_bind(m.stream)
                        .push_bind(&m.line);
                });
                qb.build().execute(&mut *tx).await?;
            }
            tx.commit().await
        }
        .await;
        if let Err(e) = result {
            tracing::error!(error = %e, "writing deployment logs");
        }
        batch.clear();
        state.deploys.touch_deployment(&deployment_id);
    }
}

/// Stop and remove every container of an app. Used by stop and delete.
pub async fn remove_app_containers(state: &AppState, app_id: &str) -> Result<()> {
    for id in state.runtime.list_by_label(LABEL_APP, app_id).await? {
        state.runtime.stop(&id, STOP_GRACE_SECS).await?;
        state.runtime.remove(&id).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_guard_threshold() {
        const GB: u64 = 1024 * 1024 * 1024;
        assert_eq!(short_of_disk(100 * GB, 10 * GB), None);
        assert_eq!(short_of_disk(100 * GB, 99 * GB + GB / 2), Some(GB / 2));
        assert_eq!(
            short_of_disk(100 * GB, 99 * GB),
            None,
            "exactly 1 GiB free is allowed"
        );
        assert_eq!(short_of_disk(0, 0), None, "unknown totals never block");
        assert_eq!(
            short_of_disk(GB, 2 * GB),
            Some(0),
            "used above total saturates"
        );
    }

    #[test]
    fn git_urls() {
        for ok in [
            "https://github.com/acme/app.git",
            "ssh://git@github.com/acme/app.git",
            "git@github.com:acme/app.git",
        ] {
            assert!(validate_git_url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "/etc",
            "file:///etc",
            "ext::sh -c id",
            "--upload-pack=touch /tmp/x",
            "-oProxyCommand=id@host:x",
            "https://",
            "https://-evil",
            "http://github.com/acme/app",
            "git@github.com:/abs",
            "https://github.com/a b",
            "https://127.0.0.1/acme/app.git",
            "https://169.254.169.254/latest/meta-data",
            "ssh://git@10.0.0.1/acme/app.git",
            "git@localhost:acme/app.git",
        ] {
            assert!(validate_git_url(bad).is_err(), "{bad}");
        }
        assert!(validate_git_url_with("https://10.0.0.1/acme/app.git", true).is_ok());
    }

    #[test]
    fn verify_no_symlinks_rejects_symlinks_in_checkout() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("README.md"), "ok").unwrap();
        assert!(verify_no_symlinks(dir.path()).is_ok());

        std::os::unix::fs::symlink("/etc/passwd", dir.path().join("Dockerfile")).unwrap();
        assert!(verify_no_symlinks(dir.path()).is_err());
    }

    #[test]
    fn relative_paths() {
        for ok in ["Dockerfile", "docker/Dockerfile.prod", "./Dockerfile"] {
            assert!(validate_relative_path(ok).is_ok(), "{ok}");
        }
        for bad in ["", "/Dockerfile", "../Dockerfile", "a/../../b"] {
            assert!(validate_relative_path(bad).is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn recover_removes_interrupted_container_and_resumes_paused_stateful_app() {
        use std::sync::{Arc, Mutex};

        #[derive(Default)]
        struct MockState {
            removed: Vec<String>,
            started: Vec<String>,
            prev_running: bool,
        }

        struct MockRuntime(Arc<Mutex<MockState>>);

        #[async_trait::async_trait]
        impl crate::runtime::Runtime for MockRuntime {
            async fn info(&self) -> Result<crate::runtime::EngineInfo> {
                Ok(crate::runtime::EngineInfo {
                    name: "docker".into(),
                    version: "27.0".into(),
                })
            }
            async fn pull(&self, _: &str, _: &LogSink) -> Result<()> {
                Ok(())
            }
            async fn build(&self, _: BuildSpec, _: &LogSink) -> Result<()> {
                Ok(())
            }
            async fn build_plan(&self, _: PlanSpec, _: &LogSink) -> Result<()> {
                Ok(())
            }
            async fn run(&self, _: &ContainerSpec) -> Result<Started> {
                bail!("unused")
            }
            async fn state(&self, id: &str) -> Result<ContainerState> {
                Ok(self.health(id).await?.map_or(ContainerState::Missing, |h| h.state))
            }
            async fn health(&self, id: &str) -> Result<Option<crate::runtime::Health>> {
                let s = self.0.lock().unwrap();
                if id == "cid-prev" {
                    Ok(Some(crate::runtime::Health {
                        state: if s.prev_running {
                            ContainerState::Running
                        } else {
                            ContainerState::Exited(Some(0))
                        },
                        restart_count: 0,
                        oom_killed: false,
                        exit_code: Some(0),
                        host_port: Some(24567),
                    }))
                } else {
                    Ok(None)
                }
            }
            async fn start(&self, id: &str) -> Result<()> {
                let mut s = self.0.lock().unwrap();
                s.started.push(id.to_string());
                if id == "cid-prev" {
                    s.prev_running = true;
                }
                Ok(())
            }
            async fn stop(&self, _: &str, _: u32) -> Result<()> {
                Ok(())
            }
            async fn remove(&self, id: &str) -> Result<()> {
                self.0.lock().unwrap().removed.push(id.to_string());
                Ok(())
            }
            fn logs_with(&self, _: &str, _: crate::runtime::LogOptions) -> crate::runtime::LogStream {
                Box::pin(futures_util::stream::empty())
            }
            async fn list_by_label(&self, label: &str, value: &str) -> Result<Vec<String>> {
                if label == LABEL_DEPLOYMENT && value == "dep-interrupted" {
                    Ok(vec!["cid-half-new".into()])
                } else {
                    Ok(vec![])
                }
            }
            async fn remove_volume(&self, _: &str) -> Result<()> {
                Ok(())
            }
            async fn remove_network(&self, _: &str) -> Result<()> {
                Ok(())
            }
            async fn shell(&self, _: &str, _: u16, _: u16) -> Result<crate::runtime::Shell> {
                bail!("unused")
            }
            async fn image_id(&self, _: &str) -> Result<Option<String>> {
                Ok(None)
            }
            async fn usage(&self, _: &str) -> Result<Option<crate::runtime::Usage>> {
                Ok(None)
            }
            async fn resize_shell(&self, _: &str, _: u16, _: u16) -> Result<()> {
                Ok(())
            }
            async fn exec(
                &self,
                _: &str,
                _: Vec<String>,
                _: Vec<String>,
                _: Option<crate::runtime::ByteReader>,
                _: crate::runtime::ByteWriter,
            ) -> Result<crate::runtime::ExecOutcome> {
                bail!("unused")
            }
        }

        struct NoopProxy;
        #[async_trait::async_trait]
        impl crate::proxy::Proxy for NoopProxy {
            async fn ready(&self) -> bool {
                true
            }
            async fn apply(
                &self,
                _: &[crate::proxy::Route],
                _: Option<&rig_api::DashboardDomain>,
                _: &[String],
            ) -> Result<()> {
                Ok(())
            }
            fn ports(&self) -> (u16, u16) {
                (80, 443)
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let db = crate::state::open_db(&format!("sqlite://{}", dir.path().join("rig.db").display()))
            .await
            .unwrap();
        let vault = crate::crypto::Vault::open(dir.path()).unwrap();
        let mock = Arc::new(Mutex::new(MockState::default()));
        let state = AppState {
            db: db.clone(),
            vault,
            config: Arc::new(crate::state::Config {
                data_dir: dir.path().to_path_buf(),
                secure_cookies: false,
                github_api: "http://127.0.0.1".into(),
                github_web: "http://127.0.0.1".into(),
                poll_interval: 0,
                allow_private_alerts: false,
                allow_private_git: false,
                enable_api_docs: false,
            }),
            runtime: Arc::new(MockRuntime(mock.clone())),
            proxy: Arc::new(NoopProxy),
            deploys: Arc::new(Deployer::new().unwrap()),
            access: crate::access::DashboardAccess::default(),
            caches: Arc::new(crate::state::RuntimeCaches::default()),
        };

        sqlx::query("INSERT INTO organizations (id, name, slug) VALUES ('org1', 'Acme', 'acme')")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO projects (id, org_id, name, slug, description, created_at) VALUES ('p1', 'org1', 'Alpha', 'alpha', '', '2026-11-22T00:00:00Z')")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO apps (id, org_id, project_id, name, slug, source_kind, image, dockerfile_path, port, status, current_deployment_id, webhook_secret, created_at, updated_at)
             VALUES ('app1', 'org1', 'p1', 'Web', 'web', 'image', 'nginx:alpine', 'Dockerfile', 80, 'deploying', 'dep-prev', 'sec', '2026-11-22T00:00:00Z', '2026-11-22T00:00:00Z')",
        )
        .execute(&db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO deployments (id, org_id, app_id, status, trigger, container_id, host_port, created_at)
             VALUES ('dep-prev', 'org1', 'app1', 'running', 'manual', 'cid-prev', 20001, '2026-11-22T00:00:00Z'),
                    ('dep-interrupted', 'org1', 'app1', 'deploying', 'manual', NULL, NULL, '2026-11-22T00:01:00Z')",
        )
        .execute(&db)
        .await
        .unwrap();

        recover(&state).await;

        {
            let s = mock.lock().unwrap();
            assert_eq!(s.removed, vec!["cid-half-new".to_string()]);
            assert_eq!(s.started, vec!["cid-prev".to_string()]);
            assert!(s.prev_running);
        }
        let app_status: String = sqlx::query_scalar("SELECT status FROM apps WHERE id = 'app1'")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(app_status, "running");

        let (dep_status, port): (String, i64) = sqlx::query_as(
            "SELECT (SELECT status FROM deployments WHERE id = 'dep-interrupted'),
                    (SELECT host_port FROM deployments WHERE id = 'dep-prev')",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!(dep_status, "failed");
        assert_eq!(port, 24567);
    }

    #[tokio::test]
    async fn prune_old_deployment_logs_keeps_latest_30_and_current() {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::state::open_db(&format!("sqlite://{}", dir.path().join("rig.db").display()))
            .await
            .unwrap();
        sqlx::query("INSERT INTO organizations (id, name, slug) VALUES ('org1', 'Acme', 'acme')")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO projects (id, org_id, name, slug, description, created_at) VALUES ('p1', 'org1', 'Alpha', 'alpha', '', '2026-11-22T00:00:00Z')")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO apps (id, org_id, project_id, name, slug, source_kind, image, dockerfile_path, port, status, current_deployment_id, webhook_secret, created_at, updated_at)
             VALUES ('app1', 'org1', 'p1', 'Web', 'web', 'image', 'nginx:alpine', 'Dockerfile', 80, 'running', 'dep-001', 'sec', '2026-11-22T00:00:00Z', '2026-11-22T00:00:00Z')",
        )
        .execute(&db)
        .await
        .unwrap();
        for i in 1..=35 {
            let id = format!("dep-{i:03}");
            let ts = format!("2026-11-22T00:{i:02}:00.000Z");
            let status = if i == 1 { "running" } else { "superseded" };
            sqlx::query(
                "INSERT INTO deployments (id, org_id, app_id, status, trigger, created_at) VALUES (?, 'org1', 'app1', ?, 'manual', ?)",
            )
            .bind(&id)
            .bind(status)
            .bind(&ts)
            .execute(&db)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO deployment_logs (deployment_id, seq, ts, stream, line) VALUES (?, 1, ?, 'build', 'line')",
            )
            .bind(&id)
            .bind(&ts)
            .execute(&db)
            .await
            .unwrap();
        }
        prune_old_deployment_logs(&db, "dep-035").await;

        let kept: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM deployment_logs")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(kept, 31);
        let current_kept: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM deployment_logs WHERE deployment_id = 'dep-001'",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!(current_kept, 1);
        let dep_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM deployments")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(dep_count, 35);
    }
}
