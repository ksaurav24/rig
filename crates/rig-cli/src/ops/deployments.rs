//! Deployment resolution, rollback target selection, and resumable log streaming
//! shared between CLI commands and `rig mcp`.

use std::collections::VecDeque;
use std::time::Duration;

use rig_api::{App, Deployment, DeploymentStatus, LogLine};

use crate::api::Client;
use crate::error::{CliError, CliResult};
use crate::output::short;

/// Minimum prefix/suffix length when matching a deployment id by partial string.
pub const MIN_DEPLOYMENT_ID_CHARS: usize = 4;

/// Picks a deployment to roll back to from an app's deployment history.
///
/// - `None`: the most recent `Superseded` deployment that still has an `image_ref`.
/// - `Some(wanted)`: exact id match wins immediately; otherwise requires at least
///   4 characters and matches a unique prefix or suffix (case-insensitive).
pub fn resolve_rollback_target<'a>(
    app_name: &str,
    deployments: &'a [Deployment],
    wanted: Option<&str>,
) -> CliResult<&'a Deployment> {
    let Some(wanted) = wanted.map(str::trim).filter(|s| !s.is_empty()) else {
        return deployments
            .iter()
            .find(|d| d.status == DeploymentStatus::Superseded && d.image_ref.is_some())
            .ok_or_else(|| {
                CliError::not_found(format!("{app_name} has no earlier deployment to roll back to"))
            });
    };

    if let Some(exact) = deployments.iter().find(|d| d.id == wanted) {
        return Ok(exact);
    }

    if wanted.chars().count() < MIN_DEPLOYMENT_ID_CHARS {
        return Err(CliError::message(format!(
            "deployment id `{wanted}` is too short; give at least {MIN_DEPLOYMENT_ID_CHARS} characters"
        )));
    }

    let needle = wanted.to_ascii_lowercase();
    let matches: Vec<&Deployment> = deployments
        .iter()
        .filter(|d| {
            let id_lower = d.id.to_ascii_lowercase();
            id_lower.starts_with(&needle) || id_lower.ends_with(&needle)
        })
        .collect();

    match matches.len() {
        0 => Err(CliError::not_found(format!(
            "{app_name} has no deployment {wanted}"
        ))),
        1 => Ok(matches[0]),
        _ => {
            let ids: Vec<String> = matches.iter().map(|d| short(&d.id)).collect();
            Err(CliError::message(format!(
                "deployment id `{wanted}` is ambiguous (matches {}). Give more characters",
                ids.join(", ")
            )))
        }
    }
}

/// Resolves the target deployment and queues a rollback for `app`.
pub async fn enqueue_rollback(
    api: &Client,
    app: &App,
    wanted_deployment: Option<&str>,
) -> CliResult<Deployment> {
    let deps = api.list_deployments(&app.id).await?;
    let target = resolve_rollback_target(&app.name, &deps, wanted_deployment)?;
    api.rollback_deployment(&target.id).await
}

/// Follows a deployment's resumable SSE log stream until `end` arrives.
///
/// Calls `on_log` with the raw JSON data payload of each `log` event and
/// returns the final status reported in the `end` event (if one arrived).
pub async fn follow_deployment(
    api: &Client,
    deployment_id: &str,
    mut on_log: impl FnMut(&str),
) -> CliResult<Option<String>> {
    let mut final_status = None;
    api.deployment_logs_stream(deployment_id, |ev| match ev.event.as_str() {
        "log" => on_log(&ev.data),
        "end" => final_status = Some(ev.data.trim_matches('"').to_string()),
        _ => {}
    })
    .await?;
    Ok(final_status)
}

/// Collects up to the last `tail` log lines of a deployment (resuming across drops)
/// until `end` arrives or `limit` elapses.
///
/// Running out of time is not an error: returns the lines seen so far and `None` status.
pub async fn collect_deployment_logs(
    api: &Client,
    deployment_id: &str,
    tail: usize,
    limit: Duration,
    mut on_progress: impl FnMut(usize),
) -> CliResult<(Vec<String>, Option<String>)> {
    let mut lines: VecDeque<String> = VecDeque::with_capacity(tail.min(256));
    let mut status = None;
    let mut total_seen: usize = 0;

    let stream = api.deployment_logs_stream(deployment_id, |ev| match ev.event.as_str() {
        "log" => {
            let line = serde_json::from_str::<LogLine>(&ev.data)
                .map(|l| l.line)
                .unwrap_or(ev.data);
            if lines.len() == tail {
                lines.pop_front();
            }
            lines.push_back(line);
            total_seen += 1;
            on_progress(total_seen);
        }
        "end" => status = Some(ev.data.trim_matches('"').to_string()),
        _ => {}
    });

    if let Ok(result) = tokio::time::timeout(limit, stream).await {
        result?;
    }
    Ok((lines.into(), status))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dep(id: &str, status: DeploymentStatus, image_ref: Option<&str>) -> Deployment {
        Deployment {
            id: id.into(),
            app_id: "app-1".into(),
            status,
            trigger: "manual".into(),
            image_ref: image_ref.map(str::to_owned),
            commit_sha: None,
            commit_msg: None,
            error: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            started_at: None,
            finished_at: None,
        }
    }

    #[test]
    fn rollback_resolution_is_strict_and_unambiguous() {
        let deps = vec![
            dep("01940000-0000-7000-8000-0000aaaa1111", DeploymentStatus::Running, Some("img:3")),
            dep("01940000-0000-7000-8000-0000bbbb1111", DeploymentStatus::Superseded, Some("img:2")),
            dep("01940000-0000-7000-8000-0000cccc2222", DeploymentStatus::Superseded, Some("img:1")),
        ];

        // Default picks newest Superseded with image.
        assert_eq!(
            resolve_rollback_target("web", &deps, None).unwrap().id,
            "01940000-0000-7000-8000-0000bbbb1111"
        );
        // Exact match works.
        assert_eq!(
            resolve_rollback_target("web", &deps, Some("01940000-0000-7000-8000-0000cccc2222"))
                .unwrap()
                .id,
            "01940000-0000-7000-8000-0000cccc2222"
        );
        // Unique suffix >= 4 chars works.
        assert_eq!(
            resolve_rollback_target("web", &deps, Some("cccc2222"))
                .unwrap()
                .id,
            "01940000-0000-7000-8000-0000cccc2222"
        );
        // Too short (< 4 chars) is rejected.
        let err = resolve_rollback_target("web", &deps, Some("1")).unwrap_err();
        assert_eq!(err.exit_code(), 1);
        assert!(err.to_string().contains("at least 4"));

        // Ambiguous suffix (both end in 1111) is rejected instead of picking first silently.
        let err = resolve_rollback_target("web", &deps, Some("1111")).unwrap_err();
        assert_eq!(err.exit_code(), 1);
        assert!(err.to_string().contains("ambiguous"));

        // Unknown id returns NotFound (exit code 4).
        let err = resolve_rollback_target("web", &deps, Some("deadbeef")).unwrap_err();
        assert_eq!(err.exit_code(), 4);
    }
}
