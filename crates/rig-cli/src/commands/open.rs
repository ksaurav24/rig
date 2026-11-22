//! `rig open [app]` — open an app's URL (or the server dashboard) in the browser.

use clap::Args;
use serde_json::json;

use crate::api::Client;
use crate::error::{CliError, CliResult};
use crate::ops;
use crate::output::{OutputOpts, domain_url, print_structured};

#[derive(Args)]
pub struct OpenArgs {
    /// App name, `project/app`, or id. Omit to open the linked app (if `rig.toml` exists) or the server dashboard.
    pub app: Option<String>,
    /// Open the Rig dashboard instead of the linked app.
    #[arg(long)]
    pub dashboard: bool,
    /// Print the URL instead of launching a browser.
    #[arg(long)]
    pub print: bool,
}

pub async fn run(api: &Client, args: OpenArgs, out: OutputOpts) -> CliResult<()> {
    let url = if args.dashboard {
        api.base().to_string()
    } else if args.app.is_some() || crate::link::discover()?.is_some() {
        let app = ops::apps::resolve_app_or_linked(api, args.app.as_deref()).await?;
        let app = api.get_app(&app.id).await?;
        if let Some(domain) = app.domains.first() {
            domain_url(domain, false)
        } else if let Some(hp) = app.host_port {
            format!("http://127.0.0.1:{hp}")
        } else {
            return Err(CliError::not_found(format!(
                "{} has no domain or running port yet. Add one with: rig domains add {} <host>",
                app.name, app.name
            )));
        }
    } else {
        api.base().to_string()
    };

    if out.is_structured() {
        return print_structured(&json!({ "url": url }), out.format);
    }
    if out.is_name() || args.print {
        println!("{url}");
        return Ok(());
    }

    open_in_browser(&url)?;
    println!("Opened {url}");
    Ok(())
}

fn open_in_browser(url: &str) -> CliResult<()> {
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    };
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", "", url]);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };

    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| CliError::message(format!("could not launch browser ({e}); open: {url}")))?;
    Ok(())
}
