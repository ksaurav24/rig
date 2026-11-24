//! Crash watcher. Docker's restart policy brings a crashed app back on its
//! own, so without this the dashboard would say "running" through a crash
//! loop or an out-of-memory kill. Every few seconds it compares each live
//! container's restart count with the last one seen and records what happened.

use std::{collections::HashSet, time::Duration};

use anyhow::Result;
use futures_util::stream::{self, StreamExt};
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
        "SELECT a.id, a.org_id, a.name, a.memory_mb, a.restart_count,
                d.id AS deployment_id, d.container_id, d.host_port
           FROM apps a JOIN deployments d ON d.id = a.current_deployment_id
          WHERE a.status = 'running' AND d.container_id IS NOT NULL",
    )
    .fetch_all(&state.db)
    .await?;

    let checked_apps: Vec<_> = stream::iter(rows)
        .map(|r| {
            let runtime = state.runtime.clone();
            async move {
                let container: String = r.get("container_id");
                let health = runtime.health(&container).await;
                (r, container, health)
            }
        })
        .buffer_unordered(8)
        .collect()
        .await;

    let mut still_down = HashSet::new();
    let mut proxy_needs_sync = false;
    for (r, container, health) in checked_apps {
        let id: String = r.get("id");
        let deployment_id: String = r.get("deployment_id");
        let stored_port: Option<i64> = r.get("host_port");
        let seen: i64 = r.get("restart_count");
        let memory = r
            .get::<Option<i64>, _>("memory_mb")
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(crate::deploy::DEFAULT_APP_MEMORY_MB);
        let health = match health {
            Ok(h) => h,
            // The engine is unreachable; that is not the app's fault.
            Err(e) => {
                tracing::debug!("inspecting {container}: {e:#}");
                continue;
            }
        };
        if let Some(h) = health.as_ref()
            && h.state == ContainerState::Running
            && let Some(live_port) = h.host_port
            && stored_port != Some(i64::from(live_port))
        {
            sqlx::query("UPDATE deployments SET host_port = ? WHERE id = ?")
                .bind(i64::from(live_port))
                .bind(&deployment_id)
                .execute(&state.db)
                .await?;
            proxy_needs_sync = true;
        }
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
    if proxy_needs_sync
        && let Err(e) = crate::proxy::sync(state).await
    {
        tracing::warn!("updating proxy after a container port change: {e:#}");
    }
    let db_rows = sqlx::query(
        "SELECT id, container_id, host_port FROM databases
          WHERE status = 'running' AND container_id IS NOT NULL",
    )
    .fetch_all(&state.db)
    .await?;
    let checked_dbs: Vec<_> = stream::iter(db_rows)
        .map(|r| {
            let runtime = state.runtime.clone();
            async move {
                let container: String = r.get("container_id");
                let health = runtime.health(&container).await;
                (r, health)
            }
        })
        .buffer_unordered(8)
        .collect()
        .await;
    for (r, health) in checked_dbs {
        let id: String = r.get("id");
        let stored_port: Option<i64> = r.get("host_port");
        if let Ok(Some(h)) = health
            && h.state == ContainerState::Running
            && let Some(live_port) = h.host_port
            && stored_port != Some(i64::from(live_port))
        {
            sqlx::query("UPDATE databases SET host_port = ? WHERE id = ?")
                .bind(i64::from(live_port))
                .bind(&id)
                .execute(&state.db)
                .await?;
        }
    }
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
            host_port: None,
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

    #[tokio::test]
    async fn watcher_reconciles_changed_container_ports_and_syncs_proxy() {
        use std::sync::{Arc, Mutex};

        struct PortShiftRuntime;
        #[async_trait::async_trait]
        impl crate::runtime::Runtime for PortShiftRuntime {
            async fn info(&self) -> anyhow::Result<crate::runtime::EngineInfo> {
                Ok(crate::runtime::EngineInfo {
                    name: "docker".into(),
                    version: "27.0".into(),
                })
            }
            async fn pull(&self, _: &str, _: &crate::runtime::LogSink) -> anyhow::Result<()> {
                Ok(())
            }
            async fn build(
                &self,
                _: crate::runtime::BuildSpec,
                _: &crate::runtime::LogSink,
            ) -> anyhow::Result<()> {
                Ok(())
            }
            async fn build_plan(
                &self,
                _: crate::runtime::PlanSpec,
                _: &crate::runtime::LogSink,
            ) -> anyhow::Result<()> {
                Ok(())
            }
            async fn run(
                &self,
                _: &crate::runtime::ContainerSpec,
            ) -> anyhow::Result<crate::runtime::Started> {
                anyhow::bail!("unused")
            }
            async fn state(&self, _: &str) -> anyhow::Result<ContainerState> {
                Ok(ContainerState::Running)
            }
            async fn health(&self, id: &str) -> anyhow::Result<Option<Health>> {
                let port = match id {
                    "cid-app" => Some(25111),
                    "cid-db" => Some(25222),
                    _ => None,
                };
                Ok(Some(Health {
                    state: ContainerState::Running,
                    restart_count: 0,
                    oom_killed: false,
                    exit_code: Some(0),
                    host_port: port,
                }))
            }
            async fn start(&self, _: &str) -> anyhow::Result<()> {
                Ok(())
            }
            async fn stop(&self, _: &str, _: u32) -> anyhow::Result<()> {
                Ok(())
            }
            async fn remove(&self, _: &str) -> anyhow::Result<()> {
                Ok(())
            }
            fn logs_with(
                &self,
                _: &str,
                _: crate::runtime::LogOptions,
            ) -> crate::runtime::LogStream {
                Box::pin(futures_util::stream::empty())
            }
            async fn list_by_label(&self, _: &str, _: &str) -> anyhow::Result<Vec<String>> {
                Ok(vec![])
            }
            async fn remove_volume(&self, _: &str) -> anyhow::Result<()> {
                Ok(())
            }
            async fn remove_network(&self, _: &str) -> anyhow::Result<()> {
                Ok(())
            }
            async fn shell(
                &self,
                _: &str,
                _: u16,
                _: u16,
            ) -> anyhow::Result<crate::runtime::Shell> {
                anyhow::bail!("unused")
            }
            async fn image_id(&self, _: &str) -> anyhow::Result<Option<String>> {
                Ok(None)
            }
            async fn usage(&self, _: &str) -> anyhow::Result<Option<crate::runtime::Usage>> {
                Ok(None)
            }
            async fn resize_shell(&self, _: &str, _: u16, _: u16) -> anyhow::Result<()> {
                Ok(())
            }
            async fn exec(
                &self,
                _: &str,
                _: Vec<String>,
                _: Vec<String>,
                _: Option<crate::runtime::ByteReader>,
                _: crate::runtime::ByteWriter,
            ) -> anyhow::Result<crate::runtime::ExecOutcome> {
                anyhow::bail!("unused")
            }
        }

        struct RecordingProxy(Arc<Mutex<Vec<crate::proxy::Route>>>);
        #[async_trait::async_trait]
        impl crate::proxy::Proxy for RecordingProxy {
            async fn ready(&self) -> bool {
                true
            }
            async fn apply(
                &self,
                routes: &[crate::proxy::Route],
                _: Option<&rig_api::DashboardDomain>,
                _: &[String],
            ) -> anyhow::Result<()> {
                *self.0.lock().unwrap() = routes.to_vec();
                Ok(())
            }
            fn ports(&self) -> (u16, u16) {
                (80, 443)
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let db = crate::state::open_db(&format!("sqlite://{}", dir.path().join("rig.db").display()))
            .await
            .unwrap();
        let vault = crate::crypto::Vault::open(dir.path()).unwrap();
        let applied_routes = Arc::new(Mutex::new(Vec::new()));
        let state = AppState {
            db: db.clone(),
            vault,
            config: Arc::new(crate::state::Config {
                data_dir: dir.path().to_path_buf(),
                secure_cookies: false,
                github_api: "http://127.0.0.1".into(),
                github_web: "http://127.0.0.1".into(),
                poll_interval: 0,
                allow_private_alerts: false,
                allow_private_git: false,
                enable_api_docs: false,
                mode: rig_api::ServerMode::Full,
            }),
            runtime: Arc::new(PortShiftRuntime),
            proxy: Arc::new(RecordingProxy(applied_routes.clone())),
            deploys: Arc::new(crate::deploy::Deployer::new().unwrap()),
            access: crate::access::DashboardAccess::default(),
            caches: Arc::new(crate::state::RuntimeCaches::default()),
            events: Arc::new(crate::events::EventBus::new()),
        };

        sqlx::query("INSERT INTO organizations (id, name, slug) VALUES ('org1', 'Acme', 'acme')")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO projects (id, org_id, name, slug, description, created_at) VALUES ('p1', 'org1', 'Alpha', 'alpha', '', '2026-11-22T00:00:00Z')")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO apps (id, org_id, project_id, name, slug, source_kind, image, dockerfile_path, port, status, current_deployment_id, webhook_secret, created_at, updated_at)
             VALUES ('app1', 'org1', 'p1', 'Web', 'web', 'image', 'nginx:alpine', 'Dockerfile', 80, 'running', 'dep1', 'sec', '2026-11-22T00:00:00Z', '2026-11-22T00:00:00Z')",
        )
        .execute(&db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO deployments (id, org_id, app_id, status, trigger, container_id, host_port, created_at)
             VALUES ('dep1', 'org1', 'app1', 'running', 'manual', 'cid-app', 20001, '2026-11-22T00:00:00Z')",
        )
        .execute(&db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO domains (id, org_id, app_id, host, tls_mode, access, created_at)
             VALUES ('dom1', 'org1', 'app1', 'app.example.com', 'off', 'public', '2026-11-22T00:00:00Z')",
        )
        .execute(&db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO databases (id, org_id, project_id, name, slug, engine, version, volume, db_name, username, password_enc, status, container_id, host_port, created_at)
             VALUES ('db1', 'org1', 'p1', 'Main', 'main', 'postgres', '17', 'rig-db-1', 'main', 'main', 'enc', 'running', 'cid-db', 20002, '2026-11-22T00:00:00Z')",
        )
        .execute(&db)
        .await
        .unwrap();

        let mut down = HashSet::new();
        check(&state, &mut down).await.unwrap();

        let (app_port, db_port): (i64, i64) = sqlx::query_as(
            "SELECT (SELECT host_port FROM deployments WHERE id = 'dep1'),
                    (SELECT host_port FROM databases WHERE id = 'db1')",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!(app_port, 25111);
        assert_eq!(db_port, 25222);

        let routes = applied_routes.lock().unwrap();
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].upstream_port, 25111);
    }
}
