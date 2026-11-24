//! `rig devices [pair|ls|revoke]` — pair and manage Rig Desktop / mobile devices.

use clap::{Args, Subcommand};
use rig_api::CreatePairingRequest;
use serde_json::json;

use crate::api::Client;
use crate::commands::tokens::RoleArg;
use crate::error::{CliError, CliResult};
use crate::output::{
    OutputOpts, confirm, label, print_names, print_structured, relative_time, short, table,
};

#[derive(Args)]
pub struct DevicesArgs {
    #[command(subcommand)]
    pub command: Option<DevicesCommand>,
}

#[derive(Subcommand)]
pub enum DevicesCommand {
    /// List paired desktop/mobile devices.
    Ls,
    /// Generate a short-lived one-time pairing code for Rig Desktop or mobile.
    Pair {
        /// Optional permission role for the paired device (defaults to your role).
        #[arg(long, value_enum)]
        role: Option<RoleArg>,
        /// Pairing code lifetime in seconds (60–900, default 300).
        #[arg(long)]
        ttl: Option<u32>,
    },
    /// Revoke a paired device by id, name, or prefix.
    Revoke {
        /// Device id, name, or prefix.
        device: String,
    },
}

pub async fn run(api: &Client, args: DevicesArgs, out: OutputOpts) -> CliResult<()> {
    match args.command {
        None | Some(DevicesCommand::Ls) => {
            let devices = api.list_devices().await?;
            if out.is_structured() {
                return print_structured(&devices, out.format);
            }
            if out.is_name() {
                let names: Vec<String> = devices.iter().map(|d| d.id.clone()).collect();
                print_names(&names);
                return Ok(());
            }
            let rows = devices
                .iter()
                .map(|d| {
                    vec![
                        short(&d.id),
                        d.name.clone(),
                        d.prefix.clone(),
                        label(&d.role),
                        d.last_ip.clone().unwrap_or_else(|| "-".into()),
                        d.last_used_at
                            .as_deref()
                            .map(relative_time)
                            .unwrap_or_else(|| "never".into()),
                    ]
                })
                .collect();
            table(
                &["ID", "DEVICE", "PREFIX", "ROLE", "LAST IP", "LAST SEEN"],
                rows,
                Some("No paired devices yet. Generate a code with: rig devices pair"),
                out.wide,
            );
        }
        Some(DevicesCommand::Pair { role, ttl }) => {
            let pairing = api
                .create_pairing(&CreatePairingRequest {
                    role: role.map(Into::into),
                    expires_in_secs: ttl,
                })
                .await?;
            if out.is_structured() {
                return print_structured(&pairing, out.format);
            }
            if out.is_name() {
                println!("{}", pairing.code);
                return Ok(());
            }
            println!("Pairing code ({}, single use):", label(&pairing.role));
            println!("\n    {}\n", pairing.code);
            println!("Expires: {}", pairing.expires_at);
            if let Some(fp) = &pairing.tls_fingerprint {
                println!("TLS SHA-256: {fp}");
            }
            println!("Enter this code in Rig Desktop -> Add server.");
        }
        Some(DevicesCommand::Revoke { device }) => {
            let devices = api.list_devices().await?;
            let needle = device.trim();
            let matches: Vec<_> = devices
                .into_iter()
                .filter(|d| {
                    d.id == needle
                        || d.name.eq_ignore_ascii_case(needle)
                        || d.prefix == needle
                        || (needle.len() >= 4 && d.id.ends_with(needle))
                })
                .collect();
            let target = match matches.len() {
                0 => {
                    return Err(CliError::not_found(format!(
                        "no paired device matching `{needle}`. List them with: rig devices ls"
                    )));
                }
                1 => matches.into_iter().next().unwrap(),
                _ => {
                    return Err(CliError::message(format!(
                        "`{needle}` matches multiple devices; pass the full id"
                    )));
                }
            };
            confirm(
                &format!("Revoke paired device {} ({})?", target.name, target.prefix),
                out.yes,
            )?;
            api.revoke_device(&target.id).await?;
            if out.is_structured() {
                return print_structured(
                    &json!({ "revoked": target.id, "name": target.name }),
                    out.format,
                );
            }
            if out.is_name() {
                println!("{}", target.id);
                return Ok(());
            }
            println!("Revoked device {} ({})", target.name, short(&target.id));
        }
    }
    Ok(())
}
