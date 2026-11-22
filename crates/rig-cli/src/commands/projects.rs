//! `rig projects [ls|create|rename|rm]`.

use clap::{Args, Subcommand, ValueEnum};
use rig_api::{CreateProjectRequest, UpdateProjectRequest};
use serde_json::json;

use crate::api::Client;
use crate::error::{CliError, CliResult};
use crate::ops;
use crate::output::{OutputOpts, confirm, print_names, print_structured, table};

#[derive(Args)]
pub struct ProjectsArgs {
    #[command(subcommand)]
    pub command: Option<ProjectCommand>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum DatabasesOnDelete {
    Move,
    Delete,
}

#[derive(Subcommand)]
pub enum ProjectCommand {
    /// List projects.
    Ls,
    /// Create a project.
    Create {
        name: String,
        /// Optional description for the project.
        #[arg(long, default_value = "")]
        description: String,
    },
    /// Rename a project or update its description.
    Rename {
        /// Project name, slug, or id.
        project: String,
        /// New project name.
        new_name: String,
        /// Optional new description.
        #[arg(long)]
        description: Option<String>,
    },
    /// Delete a project and its apps.
    Rm {
        /// Project name, slug, or id.
        project: String,
        /// What to do with databases in this project (`move` or `delete`).
        #[arg(long, value_enum)]
        databases: Option<DatabasesOnDelete>,
        /// Target project name or id when `--databases move`.
        #[arg(long)]
        move_to: Option<String>,
    },
}

pub async fn run(api: &Client, args: ProjectsArgs, out: OutputOpts) -> CliResult<()> {
    match args.command {
        None | Some(ProjectCommand::Ls) => {
            let projects = api.list_projects().await?;
            if out.is_structured() {
                return print_structured(&projects, out.format);
            }
            if out.is_name() {
                let names: Vec<String> = projects.iter().map(|p| p.name.clone()).collect();
                print_names(&names);
                return Ok(());
            }
            let rows = projects
                .iter()
                .map(|p| vec![p.name.clone(), p.app_count.to_string(), p.id.clone()])
                .collect();
            table(
                &["NAME", "APPS", "ID"],
                rows,
                Some("No projects yet. Create one with: rig projects create <name>"),
                out.wide,
            );
            Ok(())
        }
        Some(ProjectCommand::Create { name, description }) => {
            let body = CreateProjectRequest { name, description };
            let project = api.create_project(&body).await?;
            if out.is_structured() {
                return print_structured(&project, out.format);
            }
            if out.is_name() {
                println!("{}", project.name);
                return Ok(());
            }
            println!("Created project {}", project.name);
            Ok(())
        }
        Some(ProjectCommand::Rename {
            project,
            new_name,
            description,
        }) => {
            let existing = ops::projects::resolve_project(api, &project).await?;
            let updated = api
                .update_project(
                    &existing.id,
                    &UpdateProjectRequest {
                        name: Some(new_name),
                        description,
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
            println!("Renamed project {} → {}", existing.name, updated.name);
            Ok(())
        }
        Some(ProjectCommand::Rm {
            project,
            databases,
            move_to,
        }) => {
            let existing = ops::projects::resolve_project(api, &project).await?;
            confirm(
                &format!("Delete project {} and all of its apps?", existing.name),
                out.yes,
            )?;
            let db_action = databases.map(|d| match d {
                DatabasesOnDelete::Move => "move",
                DatabasesOnDelete::Delete => "delete",
            });
            let target_id = match (databases, move_to.as_deref()) {
                (Some(DatabasesOnDelete::Move), Some(t)) => {
                    Some(ops::projects::resolve_project(api, t).await?.id)
                }
                (Some(DatabasesOnDelete::Move), None) => {
                    return Err(CliError::message(
                        "--databases move requires --move-to <target-project>",
                    ));
                }
                _ => None,
            };
            api.delete_project(&existing.id, db_action, target_id.as_deref())
                .await?;
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
            println!("Deleted project {}", existing.name);
            Ok(())
        }
    }
}
