//! `rig members [ls|add|role|rm|reset-password]`.

use clap::{Args, Subcommand};
use rig_api::{AddMemberRequest, Member, UpdateMemberRequest};
use serde_json::json;

use crate::api::Client;
use crate::commands::tokens::RoleArg;
use crate::error::{CliError, CliResult};
use crate::output::{
    OutputOpts, confirm, label, print_names, print_structured, relative_time, table,
};

#[derive(Args)]
pub struct MembersArgs {
    #[command(subcommand)]
    pub command: Option<MembersCommand>,
}

#[derive(Subcommand)]
pub enum MembersCommand {
    /// List organization members.
    Ls,
    /// Add a member (prints a one-time temporary password).
    Add {
        /// Full name.
        name: String,
        /// Email address.
        email: String,
        /// Organization role.
        #[arg(long, value_enum, default_value_t = RoleArg::Developer)]
        role: RoleArg,
    },
    /// Change a member's role.
    Role {
        /// Member email or user id.
        member: String,
        /// New role.
        #[arg(value_enum)]
        role: RoleArg,
    },
    /// Remove a member from the organization.
    Rm {
        /// Member email or user id.
        member: String,
    },
    /// Reset a member's password to a one-time temporary password.
    ResetPassword {
        /// Member email or user id.
        member: String,
    },
}

async fn resolve_member(api: &Client, wanted: &str) -> CliResult<Member> {
    let needle = wanted.trim();
    let members = api.list_members().await?;
    members
        .into_iter()
        .find(|m| m.user_id == needle || m.email.eq_ignore_ascii_case(needle))
        .ok_or_else(|| {
            CliError::not_found(format!(
                "no member matching `{needle}`. List them with: rig members"
            ))
        })
}

pub async fn run(api: &Client, args: MembersArgs, out: OutputOpts) -> CliResult<()> {
    match args.command {
        None | Some(MembersCommand::Ls) => {
            let members = api.list_members().await?;
            if out.is_structured() {
                return print_structured(&members, out.format);
            }
            if out.is_name() {
                let names: Vec<String> = members.iter().map(|m| m.email.clone()).collect();
                print_names(&names);
                return Ok(());
            }
            let rows = members
                .iter()
                .map(|m| {
                    vec![
                        m.name.clone(),
                        m.email.clone(),
                        label(&m.role),
                        m.last_login_at
                            .as_deref()
                            .map(relative_time)
                            .unwrap_or_else(|| "never".into()),
                        relative_time(&m.joined_at),
                    ]
                })
                .collect();
            table(
                &["NAME", "EMAIL", "ROLE", "LAST LOGIN", "JOINED"],
                rows,
                Some("No members found."),
                out.wide,
            );
        }
        Some(MembersCommand::Add { name, email, role }) => {
            let added = api
                .add_member(&AddMemberRequest {
                    name,
                    email,
                    role: role.into(),
                })
                .await?;
            if out.is_structured() {
                return print_structured(&added, out.format);
            }
            if out.is_name() {
                println!("{}", added.member.email);
                return Ok(());
            }
            println!(
                "Added {} ({}) as {}",
                added.member.name,
                added.member.email,
                label(&added.member.role)
            );
            println!("Temporary password (shown once): {}", added.temporary_password);
        }
        Some(MembersCommand::Role { member, role }) => {
            let target = resolve_member(api, &member).await?;
            let updated = api
                .update_member(&target.user_id, &UpdateMemberRequest { role: role.into() })
                .await?;
            if out.is_structured() {
                return print_structured(&updated, out.format);
            }
            if out.is_name() {
                println!("{}", updated.email);
                return Ok(());
            }
            println!("Updated {} to {}", updated.email, label(&updated.role));
        }
        Some(MembersCommand::Rm { member }) => {
            let target = resolve_member(api, &member).await?;
            confirm(
                &format!("Remove {} ({}) from the organization?", target.name, target.email),
                out.yes,
            )?;
            api.remove_member(&target.user_id).await?;
            if out.is_structured() {
                return print_structured(
                    &json!({ "removed": target.email, "user_id": target.user_id }),
                    out.format,
                );
            }
            if out.is_name() {
                println!("{}", target.email);
                return Ok(());
            }
            println!("Removed {}", target.email);
        }
        Some(MembersCommand::ResetPassword { member }) => {
            let target = resolve_member(api, &member).await?;
            confirm(
                &format!("Reset password for {} and sign them out everywhere?", target.email),
                out.yes,
            )?;
            let reset = api.reset_member_password(&target.user_id).await?;
            if out.is_structured() {
                return print_structured(&reset, out.format);
            }
            println!("Reset password for {}", reset.member.email);
            println!("Temporary password (shown once): {}", reset.temporary_password);
        }
    }
    Ok(())
}
