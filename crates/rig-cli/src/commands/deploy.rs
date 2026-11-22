//! `rig deploy`, `rig start`, `rig restart`, `rig rollback`, and `rig stop`.

use std::io::{IsTerminal, Write};
use std::time::{Duration, Instant};

use clap::Args;
use indicatif::{ProgressBar, ProgressStyle};
use rig_api::{App, AppStatus, Deployment, LogLine};
use serde_json::json;

use crate::api::Client;
use crate::error::{CliError, CliResult};
use crate::ops;
use crate::output::{
    OutputOpts, confirm, domain_url, format_duration, parse_duration_secs, print_log,
    print_structured, short,
};

#[derive(Args)]
pub struct DeployArgs {
    /// App name or id (`project/app` if ambiguous). Defaults to `rig.toml`.
    pub app: Option<String>,
    /// Return as soon as the deployment is queued.
    #[arg(long)]
    pub no_follow: bool,
    /// If a deployment is already in progress, attach to its log stream automatically.
    #[arg(long)]
    pub attach: bool,
    /// Confirm the app reports healthy (`running`) after the deployment completes.
    #[arg(long)]
    pub wait_healthy: bool,
    /// Abort waiting after this duration (e.g. `10m`, `300s`).
    #[arg(long)]
    pub timeout: Option<String>,
}

#[derive(Args)]
pub struct StartArgs {
    /// App name, `project/app`, or id. Defaults to `rig.toml`.
    pub app: Option<String>,
    /// Return as soon as the start deployment is queued.
    #[arg(long)]
    pub no_follow: bool,
    /// Confirm the app reports healthy (`running`) after starting.
    #[arg(long)]
    pub wait_healthy: bool,
    /// Abort waiting after this duration (e.g. `5m`, `120s`).
    #[arg(long)]
    pub timeout: Option<String>,
}

#[derive(Args)]
pub struct RestartArgs {
    /// App name, `project/app`, or id. Defaults to `rig.toml`.
    pub app: Option<String>,
    /// Return as soon as the restart deployment is queued.
    #[arg(long)]
    pub no_follow: bool,
    /// Confirm the app reports healthy (`running`) after restarting.
    #[arg(long)]
    pub wait_healthy: bool,
    /// Abort waiting after this duration (e.g. `5m`, `120s`).
    #[arg(long)]
    pub timeout: Option<String>,
}

#[derive(Args)]
pub struct RollbackArgs {
    /// App name, `project/app`, or id. Defaults to `rig.toml`.
    pub app: Option<String>,
    /// Deployment id (or at least 4 characters of its prefix/suffix). Defaults to the previous live one.
    pub deployment: Option<String>,
    /// Return as soon as the rollback is queued.
    #[arg(long)]
    pub no_follow: bool,
    /// Confirm the app reports healthy (`running`) after rolling back.
    #[arg(long)]
    pub wait_healthy: bool,
    /// Abort waiting after this duration (e.g. `5m`, `120s`).
    #[arg(long)]
    pub timeout: Option<String>,
}

#[derive(Args)]
pub struct StopArgs {
    /// App name, `project/app`, or id. Defaults to `rig.toml`.
    pub app: Option<String>,
}

struct FollowOptions {
    no_follow: bool,
    wait_healthy: bool,
    timeout: Option<Duration>,
    verb: &'static str,
}

pub async fn run_deploy(api: &Client, args: DeployArgs, out: OutputOpts) -> CliResult<()> {
    let timeout = args.timeout.as_deref().map(parse_duration_secs).transpose()?;
    let app = ops::apps::resolve_app_or_linked(api, args.app.as_deref()).await?;
    let dep = match api.deploy_app(&app.id).await {
        Ok(dep) => dep,
        Err(CliError::Conflict(msg)) => {
            if let Some(active) = find_active_deployment(api, &app.id).await? {
                if args.attach {
                    if !out.is_structured() {
                        eprintln!(
                            "Attaching to in-progress deployment {} for {}",
                            short(&active.id),
                            app.name
                        );
                    }
                    active
                } else if !out.is_structured() && std::io::stdin().is_terminal() {
                    eprint!(
                        "A deployment ({}) is already in progress for {}. Attach to its logs? [Y/n]: ",
                        short(&active.id),
                        app.name
                    );
                    std::io::stderr().flush().ok();
                    let mut answer = String::new();
                    std::io::stdin().read_line(&mut answer)?;
                    let a = answer.trim().to_ascii_lowercase();
                    if a.is_empty() || a == "y" || a == "yes" {
                        active
                    } else {
                        return Err(CliError::conflict(format!(
                            "{msg}. Follow it with: rig deployments logs {}",
                            active.id
                        )));
                    }
                } else {
                    return Err(CliError::conflict(format!(
                        "{msg}. Pass --attach or follow it with: rig deployments logs {}",
                        active.id
                    )));
                }
            } else {
                return Err(CliError::conflict(msg));
            }
        }
        Err(e) => return Err(e),
    };
    after_enqueue(
        api,
        &app,
        dep,
        FollowOptions {
            no_follow: args.no_follow,
            wait_healthy: args.wait_healthy,
            timeout,
            verb: "Deploying",
        },
        out,
    )
    .await
}

pub async fn run_start(api: &Client, args: StartArgs, out: OutputOpts) -> CliResult<()> {
    let timeout = args.timeout.as_deref().map(parse_duration_secs).transpose()?;
    let app = ops::apps::resolve_app_or_linked(api, args.app.as_deref()).await?;
    let dep = api.start_app(&app.id).await?;
    after_enqueue(
        api,
        &app,
        dep,
        FollowOptions {
            no_follow: args.no_follow,
            wait_healthy: args.wait_healthy,
            timeout,
            verb: "Starting",
        },
        out,
    )
    .await
}

pub async fn run_restart(api: &Client, args: RestartArgs, out: OutputOpts) -> CliResult<()> {
    let timeout = args.timeout.as_deref().map(parse_duration_secs).transpose()?;
    let app = ops::apps::resolve_app_or_linked(api, args.app.as_deref()).await?;
    let dep = api.restart_app(&app.id).await?;
    after_enqueue(
        api,
        &app,
        dep,
        FollowOptions {
            no_follow: args.no_follow,
            wait_healthy: args.wait_healthy,
            timeout,
            verb: "Restarting",
        },
        out,
    )
    .await
}

pub async fn run_rollback(api: &Client, args: RollbackArgs, out: OutputOpts) -> CliResult<()> {
    let timeout = args.timeout.as_deref().map(parse_duration_secs).transpose()?;
    let app = ops::apps::resolve_app_or_linked(api, args.app.as_deref()).await?;
    let dep = ops::deployments::enqueue_rollback(api, &app, args.deployment.as_deref()).await?;
    after_enqueue(
        api,
        &app,
        dep,
        FollowOptions {
            no_follow: args.no_follow,
            wait_healthy: args.wait_healthy,
            timeout,
            verb: "Rolling back",
        },
        out,
    )
    .await
}

pub async fn run_stop(api: &Client, args: StopArgs, out: OutputOpts) -> CliResult<()> {
    let app = ops::apps::resolve_app_or_linked(api, args.app.as_deref()).await?;
    confirm(
        &format!("Stop {}/{}? Its container will be removed until started again.", app.project_name, app.name),
        out.yes,
    )?;
    let stopped = api.stop_app(&app.id).await?;
    if out.is_structured() {
        return print_structured(&stopped, out.format);
    }
    if out.is_name() {
        println!("{}", app.name);
        return Ok(());
    }
    println!("Stopped {}", app.name);
    Ok(())
}

async fn find_active_deployment(api: &Client, app_id: &str) -> CliResult<Option<Deployment>> {
    let deps = api.list_deployments(app_id).await?;
    Ok(deps.into_iter().find(|d| !d.status.is_finished()))
}

async fn after_enqueue(
    api: &Client,
    app: &App,
    dep: Deployment,
    opts: FollowOptions,
    out: OutputOpts,
) -> CliResult<()> {
    if opts.no_follow {
        if out.is_structured() {
            return print_structured(&dep, out.format);
        }
        if out.is_name() {
            println!("{}", dep.id);
            return Ok(());
        }
        println!("Queued deployment {} for {}", short(&dep.id), app.name);
        return Ok(());
    }

    let started = Instant::now();
    let structured = out.is_structured() || out.is_name();
    let spinner = if !structured && std::io::stderr().is_terminal() {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::with_template("{spinner:.cyan} {msg} ({elapsed})")
                .unwrap_or_else(|_| ProgressStyle::default_spinner()),
        );
        pb.set_message(format!("{} {} ({})", opts.verb, app.name, short(&dep.id)));
        pb.enable_steady_tick(Duration::from_millis(100));
        Some(pb)
    } else {
        if !structured {
            eprintln!("{} {} ({})", opts.verb, app.name, short(&dep.id));
        }
        None
    };

    let follow_fut = ops::deployments::follow_deployment(api, &dep.id, |data| {
        if structured {
            return;
        }
        if let Some(pb) = &spinner {
            if let Ok(line) = serde_json::from_str::<LogLine>(data) {
                if line.stream == "build" {
                    pb.set_message(format!("Building {} ({})", app.name, short(&dep.id)));
                } else if line.stream == "deploy" {
                    pb.set_message(format!("Deploying {} ({})", app.name, short(&dep.id)));
                }
                pb.suspend(|| print_log(data, false));
            } else {
                pb.suspend(|| print_log(data, false));
            }
        } else {
            print_log(data, false);
        }
    });

    let timed_follow = async {
        match opts.timeout {
            Some(limit) => tokio::time::timeout(limit, follow_fut)
                .await
                .map_err(|_| {
                    CliError::network(format!(
                        "timed out after {} waiting for deployment {}",
                        format_duration(limit),
                        short(&dep.id)
                    ))
                })?,
            None => follow_fut.await,
        }
    };

    let outcome = tokio::select! {
        res = timed_follow => {
            if let Some(pb) = &spinner {
                pb.finish_and_clear();
            }
            res?
        }
        _ = tokio::signal::ctrl_c() => {
            if let Some(pb) = &spinner {
                pb.finish_and_clear();
            }
            return Err(CliError::detached(format!(
                "\nDetached. Deployment {} continues on the server. Follow it with: rig deployments logs {}",
                short(&dep.id),
                dep.id
            )));
        }
    };

    let elapsed = started.elapsed();

    match outcome.status.as_deref() {
        Some("running") => {
            let mut domains = outcome.domains;
            if opts.wait_healthy {
                let remaining = opts
                    .timeout
                    .map(|t| t.saturating_sub(elapsed).max(Duration::from_secs(5)))
                    .unwrap_or(Duration::from_secs(60));
                let healthy_app = wait_app_healthy(api, &app.id, &dep.id, remaining).await?;
                if domains.is_empty() {
                    domains = healthy_app.domains;
                }
            } else if domains.is_empty()
                && let Ok(fresh) = api.get_app(&app.id).await
            {
                domains = fresh.domains;
            }

            if out.is_structured() {
                let final_dep = api.get_deployment(&dep.id).await.unwrap_or(dep);
                return print_structured(
                    &json!({
                        "deployment": final_dep,
                        "status": "running",
                        "domains": domains,
                        "duration_ms": elapsed.as_millis() as u64,
                    }),
                    out.format,
                );
            }
            if out.is_name() {
                println!("{}", dep.id);
                return Ok(());
            }
            let dur = format_duration(elapsed);
            match domains.first() {
                Some(domain) => println!("Live at {} ({dur})", domain_url(domain, false)),
                None => println!("Live ({dur}). Add a domain to reach it from outside the server"),
            }
            Ok(())
        }
        None | Some("") => Err(CliError::network(format!(
            "the log stream ended before the deployment finished. Check: rig deployments show {}",
            dep.id
        ))),
        Some(other) => {
            if out.is_structured() {
                let final_dep = api.get_deployment(&dep.id).await.unwrap_or(dep);
                let _ = print_structured(
                    &json!({
                        "deployment": final_dep,
                        "status": other,
                        "duration_ms": elapsed.as_millis() as u64,
                    }),
                    out.format,
                );
            }
            Err(CliError::deploy_failed(format!(
                "deployment {other} after {}",
                format_duration(elapsed)
            )))
        }
    }
}

async fn wait_app_healthy(
    api: &Client,
    app_id: &str,
    deployment_id: &str,
    timeout: Duration,
) -> CliResult<App> {
    let deadline = Instant::now() + timeout;
    loop {
        let app = api.get_app(app_id).await?;
        if app.status == AppStatus::Running
            && (app.current_deployment_id.as_deref() == Some(deployment_id)
                || app.current_deployment_id.is_some())
        {
            return Ok(app);
        }
        if app.status == AppStatus::Failed {
            return Err(CliError::deploy_failed(format!(
                "{} entered failed state after deploy",
                app.name
            )));
        }
        if Instant::now() >= deadline {
            return Err(CliError::network(format!(
                "timed out waiting for {} to report healthy",
                app.name
            )));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
