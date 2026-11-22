//! `rig domains ls|add|rm`.

use clap::{Args, Subcommand, ValueEnum};
use rig_api::{Access, AddDomainRequest, TlsMode};
use serde_json::json;

use crate::api::Client;
use crate::error::{CliError, CliResult};
use crate::ops;
use crate::output::{
    OutputOpts, confirm, domain_url, label, print_names, print_structured, relative_time, table,
};

#[derive(Args)]
pub struct DomainsArgs {
    #[command(subcommand)]
    pub command: DomainsCommand,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum TlsArg {
    Auto,
    Internal,
    Off,
    Tailscale,
}

impl From<TlsArg> for TlsMode {
    fn from(v: TlsArg) -> Self {
        match v {
            TlsArg::Auto => TlsMode::Auto,
            TlsArg::Internal => TlsMode::Internal,
            TlsArg::Off => TlsMode::Off,
            TlsArg::Tailscale => TlsMode::Tailscale,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum AccessArg {
    Public,
    Lan,
    Tailscale,
}

impl From<AccessArg> for Access {
    fn from(v: AccessArg) -> Self {
        match v {
            AccessArg::Public => Access::Public,
            AccessArg::Lan => Access::Lan,
            AccessArg::Tailscale => Access::Tailscale,
        }
    }
}

#[derive(Subcommand)]
pub enum DomainsCommand {
    /// List domains attached to an app.
    Ls {
        /// App name, `project/app`, or id. Defaults to `rig.toml`.
        app: Option<String>,
    },
    /// Attach a domain to an app.
    Add {
        /// App name, `project/app`, or id.
        app: String,
        /// Hostname to route (e.g. `app.example.com`).
        host: String,
        /// TLS certificate mode.
        #[arg(long, value_enum, default_value_t = TlsArg::Auto)]
        tls: TlsArg,
        /// Who may reach this domain.
        #[arg(long, value_enum, default_value_t = AccessArg::Public)]
        access: AccessArg,
        /// Optional custom listener port (1024–65535).
        #[arg(long)]
        port: Option<u16>,
    },
    /// Remove a domain from an app (by hostname or domain id).
    Rm {
        /// App name, `project/app`, or id.
        app: String,
        /// Domain hostname or id.
        domain: String,
    },
}

pub async fn run(api: &Client, args: DomainsArgs, out: OutputOpts) -> CliResult<()> {
    match args.command {
        DomainsCommand::Ls { app } => {
            let app = ops::apps::resolve_app_or_linked(api, app.as_deref()).await?;
            let domains = api.list_domains(&app.id).await?;
            if out.is_structured() {
                return print_structured(&domains, out.format);
            }
            if out.is_name() {
                let names: Vec<String> = domains.iter().map(|d| d.host.clone()).collect();
                print_names(&names);
                return Ok(());
            }
            let rows = domains
                .iter()
                .map(|d| {
                    let host_display = match d.port {
                        Some(p) => format!("{}:{p}", domain_url(&d.host, d.tls_mode == TlsMode::Off)),
                        None => domain_url(&d.host, d.tls_mode == TlsMode::Off),
                    };
                    vec![
                        host_display,
                        label(&d.tls_mode),
                        label(&d.access),
                        relative_time(&d.created_at),
                        d.id.clone(),
                    ]
                })
                .collect();
            table(
                &["URL", "TLS", "ACCESS", "CREATED", "ID"],
                rows,
                Some(&format!(
                    "No domains on {}. Add one with: rig domains add {} <host>",
                    app.name, app.name
                )),
                out.wide,
            );
        }
        DomainsCommand::Add {
            app,
            host,
            tls,
            access,
            port,
        } => {
            let app = ops::apps::resolve_app(api, &app).await?;
            let created = api
                .add_domain(
                    &app.id,
                    &AddDomainRequest {
                        host,
                        tls_mode: tls.into(),
                        access: access.into(),
                        port,
                    },
                )
                .await?;
            if out.is_structured() {
                return print_structured(&created, out.format);
            }
            if out.is_name() {
                println!("{}", created.host);
                return Ok(());
            }
            println!(
                "Added {} to {}",
                domain_url(&created.host, created.tls_mode == TlsMode::Off),
                app.name
            );
        }
        DomainsCommand::Rm { app, domain } => {
            let app = ops::apps::resolve_app(api, &app).await?;
            let domains = api.list_domains(&app.id).await?;
            let target = domains
                .into_iter()
                .find(|d| d.id == domain || d.host.eq_ignore_ascii_case(&domain))
                .ok_or_else(|| {
                    CliError::not_found(format!("no domain `{domain}` on {}", app.name))
                })?;
            confirm(
                &format!("Remove domain {} from {}?", target.host, app.name),
                out.yes,
            )?;
            api.delete_domain(&app.id, &target.id).await?;
            if out.is_structured() {
                return print_structured(
                    &json!({
                        "app": app.name,
                        "deleted": target.host,
                        "id": target.id,
                    }),
                    out.format,
                );
            }
            if out.is_name() {
                println!("{}", target.host);
                return Ok(());
            }
            println!("Removed {} from {}", target.host, app.name);
        }
    }
    Ok(())
}
