//! `rig webhook show|rotate [app]`.

use clap::{Args, Subcommand};
use serde_json::json;

use crate::api::Client;
use crate::error::CliResult;
use crate::ops;
use crate::output::{OutputOpts, confirm, print_structured};

#[derive(Args)]
pub struct WebhookArgs {
    #[command(subcommand)]
    pub command: WebhookCommand,
}

#[derive(Subcommand)]
pub enum WebhookCommand {
    /// Show an app's push-to-deploy webhook URL and signing secret (audited).
    Show {
        /// App name, `project/app`, or id. Defaults to `rig.toml`.
        app: Option<String>,
    },
    /// Rotate an app's webhook secret (old signatures stop working immediately).
    Rotate {
        /// App name, `project/app`, or id. Defaults to `rig.toml`.
        app: Option<String>,
    },
}

pub async fn run(api: &Client, args: WebhookArgs, out: OutputOpts) -> CliResult<()> {
    match args.command {
        WebhookCommand::Show { app } => {
            let app = ops::apps::resolve_app_or_linked(api, app.as_deref()).await?;
            let hook = api.get_webhook(&app.id).await?;
            let url = format!("{}{}", api.base(), hook.path);
            if out.is_structured() {
                return print_structured(
                    &json!({
                        "app": app.name,
                        "url": url,
                        "path": hook.path,
                        "secret": hook.secret,
                    }),
                    out.format,
                );
            }
            if out.is_name() {
                println!("{url}");
                return Ok(());
            }
            println!("URL     {url}");
            println!("Secret  {}", hook.secret);
        }
        WebhookCommand::Rotate { app } => {
            let app = ops::apps::resolve_app_or_linked(api, app.as_deref()).await?;
            confirm(
                &format!(
                    "Rotate webhook secret for {}? Existing webhook integrations will need the new secret.",
                    app.name
                ),
                out.yes,
            )?;
            let hook = api.rotate_webhook(&app.id).await?;
            let url = format!("{}{}", api.base(), hook.path);
            if out.is_structured() {
                return print_structured(
                    &json!({
                        "app": app.name,
                        "url": url,
                        "path": hook.path,
                        "secret": hook.secret,
                    }),
                    out.format,
                );
            }
            if out.is_name() {
                println!("{}", hook.secret);
                return Ok(());
            }
            println!("Rotated webhook secret for {}", app.name);
            println!("URL     {url}");
            println!("Secret  {}", hook.secret);
        }
    }
    Ok(())
}
