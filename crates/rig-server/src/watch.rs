//! Crash watcher. Docker's restart policy brings a crashed app back on its
//! own, so without this the dashboard would say "running" through a crash
//! loop or an out-of-memory kill. Every few seconds it compares each live
//! container's restart count with the last one seen and records what happened.

use std::{collections::HashSet, time::Duration};

use anyhow::Result;
use sqlx::Row;

use crate::{
    runtime::{ContainerState, Health},
    state::{AppState, now},
};

const CHECK_EVERY: Duration = Duration::from_secs(15);

pub async fn watcher(state: AppState) {
    let mut tick = tokio::time::interval(CHECK_EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Apps whose container was down on the previous look. Down twice in a row
    // with no restart in between means the engine has given up on it.
    let mut down = HashSet::new();
    loop {
        tick.tick().await;
        if let Err(e) = check(&state, &mut down).await {
            tracing::warn!("checking app containers: {e:#}");
        }
    }
}

/// What to record after one look at an app's container.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    Healthy,
    /// Restarted since the last look; Docker has it in hand.
    Crashed {
        reason: String,
        restarts: i64,
    },
    /// Not running and not coming back.
    Failed {
        reason: String,
    },
}

/// Decide from the engine's report alone, so the rules are unit-testable.
pub fn judge(
    health: Option<&Health>,
    seen_restarts: i64,
    was_down: bool,
    memory_mb: u32,
) -> Verdict {
    let Some(h) = health else {
        return Verdict::Failed {
            reason: "the container disappeared. Deploy again to bring the app back".into(),
        };
    };
    let reason = || {
        if h.oom_killed {
            format!("ran out of memory (limit {memory_mb} MB). Raise the memory limit in settings")
        } else {
            match h.exit_code {
                Some(c) if c != 0 => format!("exited with code {c}. Check the logs"),
                _ => "stopped unexpectedly. Check the logs".to_owned(),
            }
        }
    };
    if h.restart_count > seen_restarts {
        return Verdict::Crashed {
            reason: reason(),
            restarts: h.restart_count,
        };
    }
    match h.state {
        ContainerState::Exited(_) if was_down => Verdict::Failed { reason: reason() },
        _ => Verdict::Healthy,
    }
}

async fn check(state: &AppState, down: &mut HashSet<String>) -> Result<()> {
    let rows = sqlx::query(
        "SELECT a.id, a.org_id, a.name, a.memory_mb, a.restart_count, d.container_id
           FROM apps a JOIN deployments d ON d.id = a.current_deployment_id
          WHERE a.status = 'running' AND d.container_id IS NOT NULL",
    )
    .fetch_all(&state.db)
    .await?;
    let mut still_down = HashSet::new();
    for r in rows {
        let id: String = r.get("id");
        let container: String = r.get("container_id");
        let seen: i64 = r.get("restart_count");
        let memory = r
            .get::<Option<i64>, _>("memory_mb")
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(crate::deploy::DEFAULT_APP_MEMORY_MB);
        let health = match state.runtime.health(&container).await {
            Ok(h) => h,
            // The engine is unreachable; that is not the app's fault.
            Err(e) => {
                tracing::debug!("inspecting {container}: {e:#}");
                continue;
            }
        };
        if matches!(
            health.as_ref().map(|h| &h.state),
            Some(ContainerState::Exited(_))
        ) {
            still_down.insert(id.clone());
        }
        // A manual start resets the engine's count; follow it down so the
        // next crash is still a step up from what we stored.
        let seen = match health.as_ref() {
            Some(h) if h.restart_count < seen => {
                sqlx::query("UPDATE apps SET restart_count = ? WHERE id = ? AND restart_count = ?")
                    .bind(h.restart_count)
                    .bind(&id)
                    .bind(seen)
                    .execute(&state.db)
                    .await?;
                h.restart_count
            }
            _ => seen,
        };
        let name: String = r.get("name");
        match judge(health.as_ref(), seen, down.contains(&id), memory) {
            Verdict::Healthy => {}
            Verdict::Crashed { reason, restarts } => {
                tracing::warn!("app {name} crashed: {reason}");
                // Guarded on the status and count read above, so a deploy
                // that swapped the container meanwhile is not overwritten.
                sqlx::query(
                    "UPDATE apps SET restart_count = ?, last_crash_at = ?, last_crash_reason = ?
                      WHERE id = ? AND status = 'running' AND restart_count = ?",
                )
                .bind(restarts)
                .bind(now())
                .bind(&reason)
                .bind(&id)
                .bind(seen)
                .execute(&state.db)
                .await?;
                crate::alerts::notify(
                    state,
                    &r.get::<String, _>("org_id"),
                    crate::alerts::Alert {
                        event: rig_api::AlertEvent::AppCrashed,
                        title: format!("{name} crashed and was restarted"),
                        detail: format!("It {reason}."),
                        target: Some(id.clone()),
                    },
                );
            }
            Verdict::Failed { reason } => {
                tracing::warn!("app {name} is down: {reason}");
                let ts = now();
                sqlx::query(
                    "UPDATE apps SET status = 'failed', last_crash_at = ?, last_crash_reason = ?,
                            updated_at = ?
                      WHERE id = ? AND status = 'running' AND restart_count = ?",
                )
                .bind(&ts)
                .bind(&reason)
                .bind(&ts)
                .bind(&id)
                .bind(seen)
                .execute(&state.db)
                .await?;
                crate::alerts::notify(
                    state,
                    &r.get::<String, _>("org_id"),
                    crate::alerts::Alert {
                        event: rig_api::AlertEvent::AppDown,
                        title: format!("{name} is down"),
                        detail: format!("It {reason}."),
                        target: Some(id.clone()),
                    },
                );
                still_down.remove(&id);
            }
        }
    }
    *down = still_down;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(state: ContainerState, restarts: i64, oom: bool, code: Option<i64>) -> Health {
        Health {
            state,
            restart_count: restarts,
            oom_killed: oom,
            exit_code: code,
        }
    }

    #[test]
    fn running_with_no_new_restarts_is_healthy() {
        let x = h(ContainerState::Running, 2, false, Some(0));
        assert_eq!(judge(Some(&x), 2, false, 512), Verdict::Healthy);
    }

    #[test]
    fn a_new_restart_is_a_crash_and_names_oom() {
        let x = h(ContainerState::Running, 3, true, Some(137));
        let Verdict::Crashed { reason, restarts } = judge(Some(&x), 2, false, 256) else {
            panic!("expected a crash");
        };
        assert_eq!(restarts, 3);
        assert!(
            reason.contains("out of memory") && reason.contains("256 MB"),
            "{reason}"
        );
    }

    #[test]
    fn exit_code_is_reported_when_not_oom() {
        let x = h(ContainerState::Exited(Some(1)), 1, false, Some(1));
        let Verdict::Crashed { reason, .. } = judge(Some(&x), 0, false, 512) else {
            panic!("expected a crash");
        };
        assert!(reason.contains("code 1"), "{reason}");
    }

    #[test]
    fn down_once_waits_for_the_restart_policy() {
        let x = h(ContainerState::Exited(Some(1)), 1, false, Some(1));
        assert_eq!(judge(Some(&x), 1, false, 512), Verdict::Healthy);
        assert!(matches!(
            judge(Some(&x), 1, true, 512),
            Verdict::Failed { .. }
        ));
    }

    #[test]
    fn a_missing_container_fails() {
        assert!(matches!(judge(None, 0, false, 512), Verdict::Failed { .. }));
    }
}
