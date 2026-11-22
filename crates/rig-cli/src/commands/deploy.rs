//! `rig deploy`, `rig rollback`, and `rig stop`.

use clap::Args;
use rig_api::{App, Deployment};
use serde_json::json;

use crate::api::Client;
use crate::error::{CliError, CliResult};
use crate::ops;
use crate::output::{print_json, print_log, short};

#[derive(Args)]
pub struct DeployArgs {
    /// App name or id. Use `project/app` when the name is not unique.
    pub app: String,
    /// Return as soon as the deployment is queued.
    #[arg(long)]
    pub no_follow: bool,
}

#[derive(Args)]
pub struct RollbackArgs {
    /// App name, `project/app`, or id.
    pub app: String,
    /// Deployment id (or at least 4 characters of its prefix/suffix). Defaults to the previous live one.
    pub deployment: Option<String>,
    /// Return as soon as the rollback is queued.
    #[arg(long)]
    pub no_follow: bool,
}

#[derive(Args)]
pub struct StopArgs {
    /// App name, `project/app`, or id.
    pub app: String,
}

pub async fn run_deploy(api: &Client, args: DeployArgs, json_out: bool) -> CliResult<()> {
    let app = ops::apps::resolve_app(api, &args.app).await?;
    let dep = api.deploy_app(&app.id).await?;
    after_enqueue(api, &app, dep, args.no_follow, json_out).await
}

pub async fn run_rollback(api: &Client, args: RollbackArgs, json_out: bool) -> CliResult<()> {
    let app = ops::apps::resolve_app(api, &args.app).await?;
    let dep = ops::deployments::enqueue_rollback(api, &app, args.deployment.as_deref()).await?;
    after_enqueue(api, &app, dep, args.no_follow, json_out).await
}

pub async fn run_stop(api: &Client, args: StopArgs, json_out: bool) -> CliResult<()> {
    let app = ops::apps::resolve_app(api, &args.app).await?;
    let stopped = api.stop_app(&app.id).await?;
    if json_out {
        return print_json(&stopped);
    }
    println!("Stopped {}", app.name);
    Ok(())
}

async fn after_enqueue(
    api: &Client,
    app: &App,
    dep: Deployment,
    no_follow: bool,
    json_out: bool,
) -> CliResult<()> {
    if no_follow {
        if json_out {
            return print_json(&dep);
        }
        println!("Queued deployment {} for {}", short(&dep.id), app.name);
        return Ok(());
    }

    if !json_out {
        eprintln!("Deploying {} ({})", app.name, short(&dep.id));
    }

    let follow_fut = ops::deployments::follow_deployment(api, &dep.id, |data| {
        if !json_out {
            print_log(data, false);
        }
    });

    let result = tokio::select! {
        res = follow_fut => res?,
        _ = tokio::signal::ctrl_c() => {
            return Err(CliError::detached(format!(
                "\nDetached. Deployment {} continues on the server. Follow it with: rig deployments logs {}",
                short(&dep.id),
                dep.id
            )));
        }
    };

    match result.as_deref() {
        Some("running") => {
            let fresh = api.get_app(&app.id).await?;
            if json_out {
                let final_dep = api.get_deployment(&dep.id).await.unwrap_or(dep);
                return print_json(&json!({
                    "deployment": final_dep,
                    "status": "running",
                    "domains": fresh.domains,
                }));
            }
            match fresh.domains.first() {
                Some(domain) => println!("Live at https://{domain}"),
                None => println!("Live. Add a domain to reach it from outside the server"),
            }
            Ok(())
        }
        None | Some("") => Err(CliError::network(format!(
            "the log stream ended before the deployment finished. Check: rig deployments show {}",
            dep.id
        ))),
        Some(other) => {
            if json_out {
                let final_dep = api.get_deployment(&dep.id).await.unwrap_or(dep);
                let _ = print_json(&json!({
                    "deployment": final_dep,
                    "status": other,
                }));
            }
            Err(CliError::deploy_failed(format!("deployment {other}")))
        }
    }
}
