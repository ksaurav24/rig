//! Project resolution shared between CLI commands and MCP tools.

use rig_api::Project;

use crate::api::Client;
use crate::error::{CliError, CliResult};

/// Resolves a project by id, name, or slug (case-insensitive).
pub async fn resolve_project(api: &Client, wanted: &str) -> CliResult<Project> {
    let wanted = wanted.trim();
    let projects = api.list_projects().await?;
    projects
        .into_iter()
        .find(|p| {
            p.id == wanted
                || p.name.eq_ignore_ascii_case(wanted)
                || p.slug.eq_ignore_ascii_case(wanted)
        })
        .ok_or_else(|| {
            CliError::not_found(format!(
                "no project called {wanted}. List them with: rig projects"
            ))
        })
}
