//! Deployment resolution, rollback target selection, and resumable log streaming
//! shared between CLI commands and `rig mcp`.

use std::collections::VecDeque;
use std::time::Duration;

use rig_api::{App, Deployment, DeploymentEndEvent, DeploymentStatus, LogLine};

use crate::api::{AppLogQuery, Client};
use crate::error::{CliError, CliResult};
use crate::output::short;

/// Minimum prefix/suffix length when matching a deployment id by partial string.
pub const MIN_DEPLOYMENT_ID_CHARS: usize = 4;

/// Final outcome parsed from a deployment SSE `end` event.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeploymentOutcome {
    pub status: Option<String>,
    pub domains: Vec<String>,
}

/// Parses an SSE `end` event payload, accepting either JSON [`DeploymentEndEvent`]
/// or a plain status string (from older servers).
pub fn parse_end_event(data: &str) -> DeploymentOutcome {
    let trimmed = data.trim();
    if trimmed.starts_with('{')
        && let Ok(ev) = serde_json::from_str::<DeploymentEndEvent>(trimmed)
    {
        return DeploymentOutcome {
            status: Some(ev.status),
            domains: ev.domains,
        };
    }
    let plain = trimmed.trim_matches('"');
    DeploymentOutcome {
        status: (!plain.is_empty()).then(|| plain.to_string()),
        domains: Vec::new(),
    }
}

/// Picks a deployment to roll back to from an app's deployment history.
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
pub async fn follow_deployment(
    api: &Client,
    deployment_id: &str,
    mut on_log: impl FnMut(&str),
) -> CliResult<DeploymentOutcome> {
    let mut outcome = DeploymentOutcome::default();
    api.deployment_logs_stream(deployment_id, |ev| match ev.event.as_str() {
        "log" => on_log(&ev.data),
        "end" => outcome = parse_end_event(&ev.data),
        _ => {}
    })
    .await?;
    Ok(outcome)
}

/// Collects up to the last `tail` log lines of a deployment (resuming across drops)
/// until `end` arrives or `limit` elapses.
pub async fn collect_deployment_logs(
    api: &Client,
    deployment_id: &str,
    tail: usize,
    limit: Duration,
    mut on_progress: impl FnMut(usize),
) -> CliResult<(Vec<String>, DeploymentOutcome)> {
    let mut lines: VecDeque<String> = VecDeque::with_capacity(tail.min(256));
    let mut outcome = DeploymentOutcome::default();
    let mut total_seen: usize = 0;

    let stream =
        api.deployment_logs_stream_with_tail(deployment_id, Some(tail), |ev| match ev.event.as_str() {
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
            "end" => outcome = parse_end_event(&ev.data),
            _ => {}
        });

    if let Ok(result) = tokio::time::timeout(limit, stream).await {
        result?;
    }
    Ok((lines.into(), outcome))
}

/// Collects runtime container logs (non-following snapshot) for an app.
pub async fn collect_app_logs(
    api: &Client,
    app_id: &str,
    tail: u32,
    since: Option<String>,
    grep: Option<String>,
    limit: Duration,
) -> CliResult<Vec<String>> {
    let cap = tail.clamp(1, 2000) as usize;
    let mut lines: VecDeque<String> = VecDeque::with_capacity(cap.min(256));
    let grep_lower = grep.as_deref().map(str::to_lowercase);
    let q = AppLogQuery {
        tail: Some(tail),
        follow: Some(false),
        since,
        timestamps: Some(true),
        grep,
    };

    let stream = api.app_logs_stream(app_id, &q, |ev| {
        if ev.event != "log" {
            return;
        }
        let line = serde_json::from_str::<LogLine>(&ev.data)
            .map(|l| l.line)
            .unwrap_or(ev.data);
        if let Some(needle) = &grep_lower
            && !line.to_lowercase().contains(needle)
        {
            return;
        }
        if lines.len() == cap {
            lines.pop_front();
        }
        lines.push_back(line);
    });

    if let Ok(result) = tokio::time::timeout(limit, stream).await {
        result?;
    }
    Ok(lines.into())
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
    fn parses_plain_and_json_end_events() {
        let plain = parse_end_event("running");
        assert_eq!(plain.status.as_deref(), Some("running"));
        assert!(plain.domains.is_empty());

        let rich = parse_end_event(r#"{"status":"running","domains":["a.example.com"]}"#);
        assert_eq!(rich.status.as_deref(), Some("running"));
        assert_eq!(rich.domains, vec!["a.example.com"]);
    }

    #[test]
    fn rollback_resolution_is_strict_and_unambiguous() {
        let deps = vec![
            dep("01940000-0000-7000-8000-0000aaaa1111", DeploymentStatus::Running, Some("img:3")),
            dep("01940000-0000-7000-8000-0000bbbb1111", DeploymentStatus::Superseded, Some("img:2")),
            dep("01940000-0000-7000-8000-0000cccc2222", DeploymentStatus::Superseded, Some("img:1")),
        ];

        assert_eq!(
            resolve_rollback_target("web", &deps, None).unwrap().id,
            "01940000-0000-7000-8000-0000bbbb1111"
        );
        assert_eq!(
            resolve_rollback_target("web", &deps, Some("01940000-0000-7000-8000-0000cccc2222"))
                .unwrap()
                .id,
            "01940000-0000-7000-8000-0000cccc2222"
        );
        assert_eq!(
            resolve_rollback_target("web", &deps, Some("cccc2222"))
                .unwrap()
                .id,
            "01940000-0000-7000-8000-0000cccc2222"
        );
        let err = resolve_rollback_target("web", &deps, Some("1")).unwrap_err();
        assert_eq!(err.exit_code(), 1);
        assert!(err.to_string().contains("at least 4"));

        let err = resolve_rollback_target("web", &deps, Some("1111")).unwrap_err();
        assert_eq!(err.exit_code(), 1);
        assert!(err.to_string().contains("ambiguous"));

        let err = resolve_rollback_target("web", &deps, Some("deadbeef")).unwrap_err();
        assert_eq!(err.exit_code(), 4);
    }
}
