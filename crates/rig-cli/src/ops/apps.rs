//! App listing and resolution shared between CLI and MCP.

use rig_api::App;

use crate::api::Client;
use crate::error::{CliError, CliResult};

/// Returns true when `s` looks like a canonical UUID string (`8-4-4-4-12`).
fn looks_like_uuid(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() == 36
        && bytes[8] == b'-'
        && bytes[13] == b'-'
        && bytes[18] == b'-'
        && bytes[23] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| matches!(i, 8 | 13 | 18 | 23) || b.is_ascii_hexdigit())
}

/// Lists apps, pushing project filtering to the server via `?project_id=` when possible.
pub async fn list_apps(api: &Client, project: Option<&str>) -> CliResult<Vec<App>> {
    let Some(wanted) = project.map(str::trim).filter(|p| !p.is_empty()) else {
        return api.list_apps(None).await;
    };

    if looks_like_uuid(wanted) {
        return api.list_apps(Some(wanted)).await;
    }

    // Resolve project name/slug/id to its project_id so the server filters in SQL.
    let projects = api.list_projects().await?;
    if let Some(proj) = projects.iter().find(|p| {
        p.id == wanted || p.name.eq_ignore_ascii_case(wanted) || p.slug.eq_ignore_ascii_case(wanted)
    }) {
        return api.list_apps(Some(&proj.id)).await;
    }

    Ok(Vec::new())
}

/// Resolves `name`, `slug`, `project/name`, or an app id to a single app.
pub async fn resolve_app(api: &Client, wanted: &str) -> CliResult<App> {
    let apps = api.list_apps(None).await?;
    match_app(apps, wanted)
}

/// Pure matching logic for [`resolve_app`], separated for deterministic unit tests.
pub fn match_app(apps: Vec<App>, wanted: &str) -> CliResult<App> {
    let (project, name) = match wanted.split_once('/') {
        Some((p, n)) => (Some(p), n),
        None => (None, wanted),
    };
    let mut hits: Vec<App> = apps
        .into_iter()
        .filter(|a| {
            a.id == wanted
                || ((a.name.eq_ignore_ascii_case(name) || a.slug.eq_ignore_ascii_case(name))
                    && project.is_none_or(|p| {
                        a.project_name.eq_ignore_ascii_case(p) || a.project_id == p
                    }))
        })
        .collect();
    match hits.len() {
        0 => Err(CliError::not_found(format!(
            "no app called {wanted}. List them with: rig apps"
        ))),
        1 => Ok(hits.remove(0)),
        _ => {
            let names: Vec<String> = hits
                .iter()
                .map(|a| format!("{}/{}", a.project_name, a.name))
                .collect();
            Err(CliError::message(format!(
                "{wanted} matches several apps: {}. Use project/app",
                names.join(", ")
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig_api::{AppStatus, SourceKind};

    fn fake_app(id: &str, project_id: &str, project_name: &str, name: &str, slug: &str) -> App {
        App {
            id: id.into(),
            project_id: project_id.into(),
            project_name: project_name.into(),
            name: name.into(),
            slug: slug.into(),
            source_kind: SourceKind::Image,
            image: Some("nginx:alpine".into()),
            git_url: None,
            git_branch: None,
            dockerfile_path: "Dockerfile".into(),
            port: 80,
            memory_mb: None,
            cpu_millis: None,
            health_path: None,
            status: AppStatus::Running,
            current_deployment_id: None,
            host_port: None,
            domains: vec![],
            volumes: vec![],
            last_crash: None,
            auto_deploy: false,
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn resolves_by_name_project_or_id_and_detects_ambiguity() {
        let apps = vec![
            fake_app("app-1", "p1", "Alpha", "web", "web"),
            fake_app("app-2", "p2", "Beta", "web", "web"),
            fake_app("app-3", "p1", "Alpha", "API", "api-svc"),
        ];

        assert_eq!(match_app(apps.clone(), "api").unwrap().id, "app-3");
        assert_eq!(match_app(apps.clone(), "api-svc").unwrap().id, "app-3");
        assert_eq!(match_app(apps.clone(), "app-2").unwrap().id, "app-2");
        assert_eq!(match_app(apps.clone(), "beta/web").unwrap().id, "app-2");

        let amb = match_app(apps.clone(), "web").unwrap_err();
        assert_eq!(amb.exit_code(), 1);
        assert!(amb.to_string().contains("Alpha/web, Beta/web"));

        let missing = match_app(apps, "ghost").unwrap_err();
        assert_eq!(missing.exit_code(), 4);
    }

    #[test]
    fn detects_uuid_shape() {
        assert!(looks_like_uuid("01948b1c-8c5a-7000-8000-000000000001"));
        assert!(!looks_like_uuid("production"));
        assert!(!looks_like_uuid("01948b1c-8c5a-7000-8000-00000000000z"));
    }
}
