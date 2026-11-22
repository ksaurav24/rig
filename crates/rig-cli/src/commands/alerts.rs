//! `rig alerts [ls|add|set|rm|test]`.

use clap::{Args, Subcommand};
use rig_api::{AlertChannel, AlertEvent, CreateAlertChannelRequest, UpdateAlertChannelRequest};
use serde_json::json;

use crate::api::Client;
use crate::error::{CliError, CliResult};
use crate::output::{
    OutputOpts, confirm, label, print_names, print_structured, relative_time, short, style_status,
    table,
};

#[derive(Args)]
pub struct AlertsArgs {
    #[command(subcommand)]
    pub command: Option<AlertsCommand>,
}

#[derive(Subcommand)]
pub enum AlertsCommand {
    /// List alert channels.
    Ls,
    /// Add an alert webhook channel (prints its signing secret once).
    Add {
        /// Channel name (e.g. `ops-slack`).
        name: String,
        /// Webhook destination URL (`https://...`).
        url: String,
        /// Comma-separated events (`deploy_failed,app_crashed,app_down,disk_low,backup_failed`). Defaults to all.
        #[arg(long)]
        events: Option<String>,
    },
    /// Update an alert channel's name, events, or enabled state.
    Set {
        /// Channel name or id.
        channel: String,
        /// New channel name.
        #[arg(long)]
        name: Option<String>,
        /// Comma-separated event list.
        #[arg(long)]
        events: Option<String>,
        /// Enable the channel.
        #[arg(long, conflicts_with = "disable")]
        enable: bool,
        /// Pause/disable the channel.
        #[arg(long, conflicts_with = "enable")]
        disable: bool,
    },
    /// Remove an alert channel.
    Rm {
        /// Channel name or id.
        channel: String,
    },
    /// Send a test alert to a channel and report the outcome.
    Test {
        /// Channel name or id.
        channel: String,
    },
}

fn parse_events(raw: &str) -> CliResult<Vec<AlertEvent>> {
    let mut out = Vec::new();
    for part in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let ev = match part {
            "deploy_failed" => AlertEvent::DeployFailed,
            "app_crashed" => AlertEvent::AppCrashed,
            "app_down" => AlertEvent::AppDown,
            "disk_low" => AlertEvent::DiskLow,
            "backup_failed" => AlertEvent::BackupFailed,
            other => {
                return Err(CliError::message(format!(
                    "unknown alert event `{other}`; expected deploy_failed, app_crashed, app_down, disk_low, or backup_failed"
                )));
            }
        };
        out.push(ev);
    }
    if out.is_empty() {
        return Err(CliError::message("specify at least one alert event"));
    }
    Ok(out)
}

async fn resolve_channel(api: &Client, wanted: &str) -> CliResult<AlertChannel> {
    let needle = wanted.trim();
    let channels = api.list_alert_channels().await?;
    channels
        .into_iter()
        .find(|c| {
            c.id == needle
                || c.name.eq_ignore_ascii_case(needle)
                || (needle.len() >= 4 && c.id.ends_with(needle))
        })
        .ok_or_else(|| {
            CliError::not_found(format!(
                "no alert channel matching `{needle}`. List them with: rig alerts"
            ))
        })
}

pub async fn run(api: &Client, args: AlertsArgs, out: OutputOpts) -> CliResult<()> {
    match args.command {
        None | Some(AlertsCommand::Ls) => {
            let channels = api.list_alert_channels().await?;
            if out.is_structured() {
                return print_structured(&channels, out.format);
            }
            if out.is_name() {
                let names: Vec<String> = channels.iter().map(|c| c.name.clone()).collect();
                print_names(&names);
                return Ok(());
            }
            let rows = channels
                .iter()
                .map(|c| {
                    let events: Vec<String> = c.events.iter().map(label).collect();
                    vec![
                        short(&c.id),
                        c.name.clone(),
                        c.url_host.clone(),
                        style_status(if c.enabled { "enabled" } else { "disabled" }),
                        events.join(","),
                        c.last_sent_at
                            .as_deref()
                            .map(relative_time)
                            .unwrap_or_else(|| "never".into()),
                    ]
                })
                .collect();
            table(
                &["ID", "NAME", "HOST", "STATE", "EVENTS", "LAST SENT"],
                rows,
                Some("No alert channels yet. Add one with: rig alerts add <name> <url>"),
                out.wide,
            );
        }
        Some(AlertsCommand::Add { name, url, events }) => {
            let events = match events {
                Some(raw) => parse_events(&raw)?,
                None => AlertEvent::ALL.to_vec(),
            };
            let created = api
                .create_alert_channel(&CreateAlertChannelRequest { name, url, events })
                .await?;
            if out.is_structured() {
                return print_structured(&created, out.format);
            }
            if out.is_name() {
                println!("{}", created.channel.name);
                return Ok(());
            }
            println!(
                "Added alert channel {} ({})",
                created.channel.name, created.channel.url_host
            );
            println!("Signing secret (shown once): {}", created.signing_secret);
        }
        Some(AlertsCommand::Set {
            channel,
            name,
            events,
            enable,
            disable,
        }) => {
            let existing = resolve_channel(api, &channel).await?;
            let events = events.as_deref().map(parse_events).transpose()?;
            let enabled = if enable {
                Some(true)
            } else if disable {
                Some(false)
            } else {
                None
            };
            let updated = api
                .update_alert_channel(
                    &existing.id,
                    &UpdateAlertChannelRequest {
                        name,
                        events,
                        enabled,
                    },
                )
                .await?;
            if out.is_structured() {
                return print_structured(&updated, out.format);
            }
            if out.is_name() {
                println!("{}", updated.name);
                return Ok(());
            }
            println!(
                "Updated alert channel {} ({})",
                updated.name,
                style_status(if updated.enabled { "enabled" } else { "disabled" })
            );
        }
        Some(AlertsCommand::Rm { channel }) => {
            let existing = resolve_channel(api, &channel).await?;
            confirm(&format!("Remove alert channel {}?", existing.name), out.yes)?;
            api.delete_alert_channel(&existing.id).await?;
            if out.is_structured() {
                return print_structured(
                    &json!({ "deleted": existing.name, "id": existing.id }),
                    out.format,
                );
            }
            if out.is_name() {
                println!("{}", existing.name);
                return Ok(());
            }
            println!("Removed alert channel {}", existing.name);
        }
        Some(AlertsCommand::Test { channel }) => {
            let existing = resolve_channel(api, &channel).await?;
            let result = api.test_alert_channel(&existing.id).await?;
            if out.is_structured() {
                return print_structured(&result, out.format);
            }
            if result.ok {
                println!("Test alert delivered to {}", existing.name);
            } else {
                let err = result.error.unwrap_or_else(|| "delivery failed".into());
                return Err(CliError::message(format!(
                    "test alert to {} failed: {err}",
                    existing.name
                )));
            }
        }
    }
    Ok(())
}
