//! `rig projects` and `rig projects create <name>`.

use clap::{Args, Subcommand};
use rig_api::CreateProjectRequest;

use crate::api::Client;
use crate::error::CliResult;
use crate::output::{print_json, table};

#[derive(Args)]
pub struct ProjectsArgs {
    #[command(subcommand)]
    pub command: Option<ProjectCommand>,
}

#[derive(Subcommand)]
pub enum ProjectCommand {
    /// Create a project.
    Create {
        name: String,
        /// Optional description for the project.
        #[arg(long, default_value = "")]
        description: String,
    },
}

pub async fn run(api: &Client, args: ProjectsArgs, json_out: bool, wide: bool) -> CliResult<()> {
    match args.command {
        None => {
            let projects = api.list_projects().await?;
            if json_out {
                return print_json(&projects);
            }
            let rows = projects
                .iter()
                .map(|p| vec![p.name.clone(), p.app_count.to_string(), p.id.clone()])
                .collect();
            table(
                &["NAME", "APPS", "ID"],
                rows,
                Some("No projects yet. Create one with: rig projects create <name>"),
                wide,
            );
            Ok(())
        }
        Some(ProjectCommand::Create { name, description }) => {
            let body = CreateProjectRequest { name, description };
            let project = api.create_project(&body).await?;
            if json_out {
                return print_json(&project);
            }
            println!("Created project {}", project.name);
            Ok(())
        }
    }
}
