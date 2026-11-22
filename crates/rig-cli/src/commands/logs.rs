//! `rig logs <app>`.

use clap::Args;

use crate::api::Client;
use crate::error::{CliError, CliResult};
use crate::ops;
use crate::output::print_log;

#[derive(Args)]
pub struct LogsArgs {
    /// App name, `project/app`, or id.
    pub app: String,
}

pub async fn run(api: &Client, args: LogsArgs, json_out: bool) -> CliResult<()> {
    let app = ops::apps::resolve_app(api, &args.app).await?;
    if app.current_deployment_id.is_none() {
        return Err(CliError::message(format!(
            "{} is not running, so it has no logs. Deploy it first",
            app.name
        )));
    }
    let stream = api.app_logs_stream(&app.id, |ev| {
        if ev.event == "log" {
            print_log(&ev.data, json_out);
        }
    });
    tokio::select! {
        res = stream => res,
        _ = tokio::signal::ctrl_c() => Ok(()),
    }
}
