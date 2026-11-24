//! `rig link [app]` — write or inspect a repo-local `rig.toml`.

use clap::Args;
use serde_json::json;

use crate::api::Client;
use crate::error::{CliError, CliResult};
use crate::link::{self, RigLink};
use crate::ops;
use crate::output::{OutputOpts, print_structured};

#[derive(Args)]
pub struct LinkArgs {
    /// App name, `project/app`, or id to link this directory to. Omit to show the current link.
    pub app: Option<String>,
}

pub async fn run(
    api: &Client,
    args: LinkArgs,
    active_context: Option<String>,
    out: OutputOpts,
) -> CliResult<()> {
    let Some(wanted) = args.app else {
        let Some((path, existing)) = link::discover()? else {
            return Err(CliError::not_found(
                "this directory is not linked to an app. Run: rig link <app>",
            ));
        };
        if out.is_structured() {
            return print_structured(
                &json!({
                    "path": path.display().to_string(),
                    "link": existing,
                }),
                out.format,
            );
        }
        if out.is_name() {
            println!("{}", existing.app_ref());
            return Ok(());
        }
        println!("Linked to {} ({})", existing.app_ref(), path.display());
        return Ok(());
    };

    let app = ops::apps::resolve_app(api, &wanted).await?;
    let link = RigLink {
        app: app.slug.clone(),
        project: Some(app.project_name.clone()),
        context: active_context,
        server: Some(api.base().to_string()),
        ..Default::default()
    };
    let cwd = std::env::current_dir()?;
    let path = link::save(&cwd, &link)?;

    if out.is_structured() {
        return print_structured(
            &json!({
                "path": path.display().to_string(),
                "app": app,
                "link": link,
            }),
            out.format,
        );
    }
    if out.is_name() {
        println!("{}/{}", app.project_name, app.name);
        return Ok(());
    }
    println!(
        "Linked {} to {}/{}",
        path.display(),
        app.project_name,
        app.name
    );
    Ok(())
}
