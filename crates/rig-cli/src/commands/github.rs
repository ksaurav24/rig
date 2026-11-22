//! `rig github [status|repos|branches]`.

use clap::{Args, Subcommand};

use crate::api::Client;
use crate::error::{CliError, CliResult};
use crate::output::{OutputOpts, print_names, print_structured, table};

#[derive(Args)]
pub struct GithubArgs {
    #[command(subcommand)]
    pub command: Option<GithubCommand>,
}

#[derive(Subcommand)]
pub enum GithubCommand {
    /// Show the organization's connected GitHub App status.
    Status,
    /// List repositories the GitHub App is installed on.
    Repos,
    /// List branches of a connected repository (`owner/repo`).
    Branches {
        /// Repository in `owner/repo` format.
        repo: String,
    },
}

pub async fn run(api: &Client, args: GithubArgs, out: OutputOpts) -> CliResult<()> {
    match args.command {
        None | Some(GithubCommand::Status) => {
            let status = api.github_status().await?;
            if out.is_structured() {
                return print_structured(&status, out.format);
            }
            match &status.app {
                Some(app) => {
                    if out.is_name() {
                        println!("{}", app.slug);
                        return Ok(());
                    }
                    println!("App          {} ({})", app.name, app.slug);
                    println!("Owner        {}", app.owner);
                    println!("URL          {}", app.html_url);
                    println!("Install URL  {}", app.install_url);
                    println!("Webhook      {}{}", api.base(), status.webhook_url);
                    println!("Poll         {}s", status.poll_interval);
                }
                None => {
                    println!("No GitHub App connected. Connect one in the Rig dashboard.");
                }
            }
        }
        Some(GithubCommand::Repos) => {
            let repos = api.github_repos().await?;
            if out.is_structured() {
                return print_structured(&repos, out.format);
            }
            if out.is_name() {
                let names: Vec<String> = repos.iter().map(|r| r.full_name.clone()).collect();
                print_names(&names);
                return Ok(());
            }
            let rows = repos
                .iter()
                .map(|r| {
                    vec![
                        r.full_name.clone(),
                        r.default_branch.clone(),
                        if r.private { "private" } else { "public" }.into(),
                        r.clone_url.clone(),
                    ]
                })
                .collect();
            table(
                &["REPOSITORY", "DEFAULT BRANCH", "VISIBILITY", "CLONE URL"],
                rows,
                Some("No GitHub repositories found."),
                out.wide,
            );
        }
        Some(GithubCommand::Branches { repo }) => {
            let Some((owner, name)) = repo.split_once('/') else {
                return Err(CliError::message("expected repository as `owner/repo`"));
            };
            let branches = api.github_branches(owner, name).await?;
            if out.is_structured() {
                return print_structured(&branches, out.format);
            }
            if out.is_name() {
                let names: Vec<String> = branches.iter().map(|b| b.name.clone()).collect();
                print_names(&names);
                return Ok(());
            }
            let rows = branches
                .iter()
                .map(|b| {
                    vec![
                        b.name.clone(),
                        if b.default { "yes" } else { "" }.into(),
                        if b.protected { "yes" } else { "" }.into(),
                    ]
                })
                .collect();
            table(
                &["BRANCH", "DEFAULT", "PROTECTED"],
                rows,
                Some("No branches found."),
                out.wide,
            );
        }
    }
    Ok(())
}
