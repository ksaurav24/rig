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
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use futures_util::StreamExt;
use sqlx::Row;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    sync::{Notify, mpsc, watch},
};

use crate::{
    proxy,
    runtime::{BuildSpec, ContainerSpec, ContainerState, LogMsg, LogSink, PlanSpec, Started},
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

/// Wakes the worker and the log followers.
pub struct Deployer {
    queued: Notify,
    /// Bumped whenever log lines or a deployment status are written.
    activity: watch::Sender<u64>,
    http: reqwest::Client,
    /// Active deployment cancellation handles keyed by deployment id.
    cancels: std::sync::Mutex<HashMap<String, watch::Sender<bool>>>,
}

impl Deployer {
    pub fn new() -> Result<Self> {
        Ok(Self {
            queued: Notify::new(),
            activity: watch::channel(0).0,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(3))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .context("building health check client")?,
            cancels: std::sync::Mutex::new(HashMap::new()),
        })
    }

    /// A receiver that changes whenever deployment state or logs change.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.activity.subscribe()
    }

    pub fn touch(&self) {
        self.activity.send_modify(|n| *n = n.wrapping_add(1));
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
}

/// Queue a deployment and wake the worker. Returns the deployment id.
pub async fn enqueue(state: &AppState, d: NewDeployment<'_>) -> Result<String, sqlx::Error> {
    let id = new_id();
    sqlx::query(
        "INSERT INTO deployments (id, org_id, app_id, status, trigger, image_ref, image_id, created_by, created_at)
         VALUES (?, ?, ?, 'queued', ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(d.org_id)
    .bind(d.app_id)
    .bind(d.trigger)
    .bind(d.image_ref)
    .bind(d.image_id)
    .bind(d.created_by)
    .bind(now())
    .execute(&state.db)
    .await?;
    state.deploys.queued.notify_one();
    state.deploys.touch();
    Ok(id)
}

/// Run the deployment worker until the process exits.
pub async fn worker(state: AppState) {
    // Work that was in flight when the last process stopped cannot resume:
    // its build context and log channel are gone.
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
    if let Err(e) = proxy::sync(&state).await {
        tracing::warn!(error = %format!("{e:#}"), "proxy not updated at startup");
    }

    loop {
        let next = sqlx::query_scalar::<_, String>(
            "SELECT id FROM deployments WHERE status = 'queued' ORDER BY created_at, id LIMIT 1",
        )
        .fetch_optional(&state.db)
        .await;
        match next {
            Ok(Some(id)) => run(&state, &id).await,
            Ok(None) => state.deploys.queued.notified().await,
            Err(e) => {
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
}

/// An image fixed when the deployment was queued (a rollback).
struct Preset {
    image_ref: Option<String>,
    image_id: Option<String>,
}

async fn load_app(state: &AppState, deployment_id: &str) -> Result<(AppRow, Preset)> {
    let r = sqlx::query(
        "SELECT a.id, a.slug, a.source_kind, a.image, a.git_url, a.git_branch, a.dockerfile_path,
                a.port, a.memory_mb, a.cpu_millis, a.health_path, a.current_deployment_id,
                a.project_id, a.org_id, d.image_ref, d.image_id
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
    };
    let preset = Preset {
        image_ref: r.get("image_ref"),
        image_id: r.get("image_id"),
    };
    Ok((app, preset))
}

async fn run(state: &AppState, deployment_id: &str) {
    let (tx, rx) = mpsc::channel::<LogMsg>(1024);
    let log = LogSink::new(tx);
    let writer = tokio::spawn(write_logs(state.clone(), deployment_id.to_string(), rx));
    let mut cancel_rx = state.deploys.register_cancel(deployment_id);

    let cancelled;
    let outcome = tokio::select! {
        res = tokio::time::timeout(DEPLOY_TIMEOUT, pipeline(state, deployment_id, &log)) => {
            cancelled = false;
            match res {
                Ok(r) => r,
                Err(_) => Err(anyhow!(
                    "the deployment took longer than 30 minutes and was stopped"
                )),
            }
        }
        _ = cancel_rx.wait_for(|&c| c) => {
            cancelled = true;
            Ok(())
        }
    };
    state.deploys.unregister_cancel(deployment_id);

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
    state.deploys.touch();
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
    state.deploys.touch();
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
    let dir = state.config.data_dir.clone();
    let usage = tokio::task::spawn_blocking(move || crate::metrics::sample_host(&dir)).await??;
    if let Some(free) = short_of_disk(usage.disk_total, usage.disk_used) {
        let engine = state
            .runtime
            .info()
            .await
            .map(|e| e.name)
            .unwrap_or_else(|_| "docker".into());
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
    ensure_disk_space(state).await?;

    // ---- image
    // A rollback runs the exact image it ran before, if the engine still has it.
    let pinned = match &preset.image_id {
        Some(id) => state.runtime.image_id(id).await?,
        None => None,
    };
    let image = match (preset.image_ref, app.source_kind.as_str()) {
        (Some(image), _) if pinned.is_some() => {
            log.line("build", format!("Reusing image {image}")).await;
            image
        }
        (Some(image), "git") => {
            log.line("build", format!("Reusing image {image}")).await;
            image
        }
        (Some(image), _) => {
            log.line("build", format!("Pulling {image}")).await;
            state.runtime.pull(&image, log).await?;
            image
        }
        (None, "git") => {
            let tag = format!("rig/{}:{short}", app.slug);
            build_from_git(state, deployment_id, &app, &tag, log).await?;
            tag
        }
        (None, _) => {
            let image = app.image.clone().context("the app has no image set")?;
            log.line("build", format!("Pulling {image}")).await;
            state.runtime.pull(&image, log).await?;
            image
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
        command: None,
        // Shared with the project's databases, so apps reach them by name.
        network: Some(project_network(&app.project_id)),
        aliases: vec![app.slug.clone()],
        volumes: volumes.clone(),
    };

    // Two containers must never write one volume, so stateful apps stop the
    // old container first and accept a few seconds of downtime.
    let mut paused = Vec::new();
    if !volumes.is_empty() {
        for id in state.runtime.list_by_label(LABEL_APP, &app.id).await? {
            log.line(
                "deploy",
                "Stopping the running container: this app has volumes",
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

/// `(engine volume name, mount path)` pairs for an app.
async fn app_volumes(state: &AppState, app_id: &str) -> Result<Vec<(String, String)>> {
    Ok(sqlx::query_as(
        "SELECT name, mount_path FROM app_volumes WHERE app_id = ? ORDER BY mount_path",
    )
    .bind(app_id)
    .fetch_all(&state.db)
    .await?)
}

/// Delete an app's volumes. Call after its containers are gone.
pub async fn remove_app_volumes(state: &AppState, app_id: &str) -> Result<()> {
    for (name, _) in app_volumes(state, app_id).await? {
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

/// Wait until the new container answers, or explain why it does not.
async fn wait_healthy(
    state: &AppState,
    started: &Started,
    app: &AppRow,
    log: &LogSink,
) -> Result<()> {
    let what = match &app.health_path {
        Some(path) => format!("GET {path} on port {}", app.port),
        None => format!("port {}", app.port),
    };
    log.line("deploy", format!("Waiting for {what}")).await;
    let begin = Instant::now();
    // An engine's port forwarder can accept connections before the process
    // listens, so one success is not enough: the container must also still
    // be running on the next look.
    let mut passes = 0;
    loop {
        match state.runtime.state(&started.id).await? {
            ContainerState::Exited(code) => bail!(
                "the container exited{} before it became healthy. Its last output is above",
                code.map(|c| format!(" with code {c}")).unwrap_or_default()
            ),
            ContainerState::Missing => bail!("the container disappeared before it became healthy"),
            ContainerState::Running | ContainerState::Other(_) => {}
        }
        let ok = match &app.health_path {
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
                "nothing answered on {what} within {}s. Check that the app listens on 0.0.0.0:{} \
                 (not 127.0.0.1) and that the port setting matches",
                HEALTH_TIMEOUT.as_secs(),
                app.port
            );
        }
        tokio::time::sleep(Duration::from_millis(700)).await;
    }
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
    match tokio::time::timeout(Duration::from_millis(300), stream.read(&mut byte)).await {
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
    validate_git_url(url).map_err(|m| anyhow!(m))?;
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
    // Arguments are passed as an argument vector, never through a shell, and
    // `--` ends option parsing before the URL.
    cmd.args([
        "-c",
        "protocol.file.allow=never",
        "-c",
        "protocol.ext.allow=never",
    ])
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

    if !tokio::fs::try_exists(checkout.join(&app.dockerfile_path))
        .await
        .unwrap_or(false)
    {
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

/// Accept only remote repositories over HTTPS or SSH.
///
/// Local paths and the `file`/`ext` transports would let a repository URL
/// read files on the server or run a command.
pub fn validate_git_url(url: &str) -> Result<(), &'static str> {
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
        return if rest.is_empty() || rest.starts_with('/') || rest.starts_with('-') {
            Err("the repository URL has no host")
        } else {
            Ok(())
        };
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

/// Persist log lines in batches so a chatty build costs few transactions.
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
            for m in &batch {
                seq += 1;
                sqlx::query(
                    "INSERT INTO deployment_logs (deployment_id, seq, ts, stream, line) VALUES (?, ?, ?, ?, ?)",
                )
                .bind(&deployment_id)
                .bind(seq)
                .bind(&ts)
                .bind(m.stream)
                .bind(&m.line)
                .execute(&mut *tx)
                .await?;
            }
            tx.commit().await
        }
        .await;
        if let Err(e) = result {
            tracing::error!(error = %e, "writing deployment logs");
        }
        batch.clear();
        state.deploys.touch();
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
        ] {
            assert!(validate_git_url(bad).is_err(), "{bad}");
        }
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
}
