//! Managed databases: one container and one named volume each, attached to
//! the project's network so the project's apps reach it by name.
//!
//! Stopping removes the container and keeps the volume; starting provisions a
//! fresh container on the same volume. That keeps one code path for "create"
//! and "start" and never depends on a stopped container still existing.

use std::{collections::HashMap, time::Duration};

use anyhow::{Context, Result, bail};
use rig_api::DatabaseEngine;
use sqlx::Row;
use tokio::{sync::mpsc, time::Instant};

use crate::{
    deploy::{self, LABEL_MANAGED},
    runtime::{ContainerSpec, ContainerState, LogSink},
    state::AppState,
};

pub const LABEL_DATABASE: &str = "rig.database";

/// First boot initialises the data directory, which is slow on small disks.
const READY_TIMEOUT: Duration = Duration::from_secs(180);
const STOP_GRACE_SECS: u32 = 30;

/// Every database gets a memory ceiling, so one runaway query cannot take
/// the whole server down.
pub const DEFAULT_DATABASE_MEMORY_MB: u32 = 1024;

/// What Rig knows about running one engine.
pub struct EngineSpec {
    pub engine: DatabaseEngine,
    pub label: &'static str,
    /// Offered versions, newest first.
    pub versions: &'static [&'static str],
    pub port: u16,
    /// Where the engine keeps its data inside the container.
    data_dir: &'static str,
}

pub const ENGINES: [EngineSpec; 5] = [
    EngineSpec {
        engine: DatabaseEngine::Postgres,
        label: "PostgreSQL",
        versions: &["17", "16", "15"],
        port: 5432,
        data_dir: "/var/lib/postgresql/data",
    },
    EngineSpec {
        engine: DatabaseEngine::Mysql,
        label: "MySQL",
        versions: &["8.4", "8.0"],
        port: 3306,
        data_dir: "/var/lib/mysql",
    },
    EngineSpec {
        engine: DatabaseEngine::Mariadb,
        label: "MariaDB",
        versions: &["11.4", "10.11"],
        port: 3306,
        data_dir: "/var/lib/mysql",
    },
    EngineSpec {
        engine: DatabaseEngine::Redis,
        label: "Redis",
        versions: &["7.4", "7.2"],
        port: 6379,
        data_dir: "/data",
    },
    EngineSpec {
        engine: DatabaseEngine::Mongodb,
        label: "MongoDB",
        versions: &["8.0", "7.0"],
        port: 27017,
        data_dir: "/data/db",
    },
];

pub fn engine_spec(engine: DatabaseEngine) -> &'static EngineSpec {
    ENGINES
        .iter()
        .find(|e| e.engine == engine)
        .expect("every engine has a spec")
}

impl EngineSpec {
    fn image(&self, version: &str) -> String {
        match self.engine {
            DatabaseEngine::Postgres => format!("postgres:{version}-alpine"),
            DatabaseEngine::Mysql => format!("mysql:{version}"),
            DatabaseEngine::Mariadb => format!("mariadb:{version}"),
            DatabaseEngine::Redis => format!("redis:{version}-alpine"),
            DatabaseEngine::Mongodb => format!("mongo:{version}"),
        }
    }

    fn env(&self, user: &str, password: &str, db: &str) -> Vec<(String, String)> {
        let pairs: &[(&str, &str)] = match self.engine {
            DatabaseEngine::Postgres => &[
                ("POSTGRES_USER", user),
                ("POSTGRES_PASSWORD", password),
                ("POSTGRES_DB", db),
            ],
            DatabaseEngine::Mysql => &[
                ("MYSQL_DATABASE", db),
                ("MYSQL_USER", user),
                ("MYSQL_PASSWORD", password),
                ("MYSQL_RANDOM_ROOT_PASSWORD", "yes"),
            ],
            DatabaseEngine::Mariadb => &[
                ("MARIADB_DATABASE", db),
                ("MARIADB_USER", user),
                ("MARIADB_PASSWORD", password),
                ("MARIADB_RANDOM_ROOT_PASSWORD", "yes"),
            ],
            // The user lives in `admin`; the init script names the app's database.
            DatabaseEngine::Mongodb => &[
                ("MONGO_INITDB_ROOT_USERNAME", user),
                ("MONGO_INITDB_ROOT_PASSWORD", password),
                ("MONGO_INITDB_DATABASE", db),
            ],
            // Pass the Redis password in the container environment and feed it
            // via stdin to `redis-server -` so it never appears in container
            // argv or host `/proc/<pid>/cmdline`.
            DatabaseEngine::Redis => &[("REDIS_PASSWORD", password)],
        };
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn command(&self) -> Option<Vec<String>> {
        match self.engine {
            DatabaseEngine::Redis => Some(
                [
                    "sh",
                    "-c",
                    r#"printf 'requirepass %s\n' "$REDIS_PASSWORD" | exec docker-entrypoint.sh redis-server - --appendonly yes"#,
                ]
                .map(String::from)
                .to_vec(),
            ),
            _ => None,
        }
    }

    /// Connection URL as seen from `host`.
    pub fn url(&self, user: &str, password: &str, host: &str, port: u16, db: &str) -> String {
        let scheme = match self.engine {
            DatabaseEngine::Postgres => "postgres",
            DatabaseEngine::Mysql | DatabaseEngine::Mariadb => "mysql",
            DatabaseEngine::Redis => "redis",
            DatabaseEngine::Mongodb => "mongodb",
        };
        let query = if self.engine == DatabaseEngine::Mongodb {
            "?authSource=admin"
        } else {
            ""
        };
        format!("{scheme}://{user}:{password}@{host}:{port}/{db}{query}")
    }
}

/// Role and database name derived from a slug: letters, digits, underscores,
/// starting with a letter. Redis has a fixed user and numbered databases.
pub fn identifiers(engine: DatabaseEngine, slug: &str) -> (String, String) {
    if engine == DatabaseEngine::Redis {
        return ("default".into(), "0".into());
    }
    let mut ident: String = slug
        .chars()
        .map(|c| if c == '-' { '_' } else { c })
        .collect();
    if !ident.starts_with(|c: char| c.is_ascii_lowercase()) {
        ident.insert_str(0, "db_");
    }
    ident.truncate(32);
    (ident.clone(), ident)
}

/// A password that is safe in a URL, an environment variable and an argv
/// slot without quoting (192 bits, hex).
pub fn new_password() -> String {
    let mut buf = [0u8; 24];
    rand::fill(&mut buf[..]);
    hex::encode(buf)
}

pub fn volume_name(id: &str) -> String {
    format!("rig-db-{}", &id[id.len().saturating_sub(12)..])
}

struct Row_ {
    project_id: String,
    slug: String,
    engine: DatabaseEngine,
    version: String,
    volume: String,
    db_name: String,
    username: String,
    password_enc: String,
}

async fn load(state: &AppState, id: &str) -> Result<Row_> {
    let r = sqlx::query(
        "SELECT project_id, slug, engine, version, volume, db_name, username, password_enc
           FROM databases WHERE id = ?",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await
    .context("loading the database")?;
    let engine: String = r.get("engine");
    Ok(Row_ {
        project_id: r.get("project_id"),
        slug: r.get("slug"),
        engine: serde_json::from_value(serde_json::Value::String(engine))
            .context("unknown engine")?,
        version: r.get("version"),
        volume: r.get("volume"),
        db_name: r.get("db_name"),
        username: r.get("username"),
        password_enc: r.get("password_enc"),
    })
}

/// Bring a database in the `creating` state up. Runs in its own task; the
/// outcome is written to the row.
pub async fn provision(state: AppState, id: String) {
    let outcome = start(&state, &id).await;
    let result = match outcome {
        Ok((container_id, host_port)) => sqlx::query(
            "UPDATE databases SET status = 'running', error = NULL, container_id = ?, host_port = ?
                  WHERE id = ?",
        )
        .bind(&container_id)
        .bind(i64::from(host_port))
        .bind(&id)
        .execute(&state.db)
        .await,
        Err(e) => {
            tracing::warn!(database = %id, error = %format!("{e:#}"), "database did not start");
            sqlx::query(
                "UPDATE databases SET status = 'failed', error = ?, container_id = NULL, host_port = NULL
                  WHERE id = ?",
            )
            .bind(format!("{e:#}"))
            .bind(&id)
            .execute(&state.db)
            .await
        }
    };
    if let Err(e) = result {
        tracing::error!(database = %id, error = %e, "could not record database state");
    }
}

async fn start(state: &AppState, id: &str) -> Result<(String, u16)> {
    let row = load(state, id).await?;
    let spec = engine_spec(row.engine);
    let password = state
        .vault
        .decrypt(&row.password_enc)
        .context("decrypting the database password")?;
    let image = spec.image(&row.version);

    // A leftover container from an earlier attempt would hold the name.
    remove_containers(state, id).await?;

    // Pull progress has nowhere to go yet; drain it so the pull never blocks.
    let (tx, mut rx) = mpsc::channel(64);
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let pulled = state.runtime.pull(&image, &LogSink::new(tx)).await;
    drain.abort();
    pulled.with_context(|| format!("pulling {image}"))?;

    let short = &id[id.len().saturating_sub(8)..];
    let container = ContainerSpec {
        name: format!("rig-db-{}-{short}", row.slug),
        image,
        port: spec.port,
        env: spec.env(&row.username, &password, &row.db_name),
        labels: HashMap::from([
            (LABEL_MANAGED.to_string(), "true".to_string()),
            (LABEL_DATABASE.to_string(), id.to_string()),
        ]),
        memory_mb: Some(DEFAULT_DATABASE_MEMORY_MB),
        cpu_millis: None,
        command: spec.command(),
        network: Some(deploy::project_network(&row.project_id)),
        aliases: vec![row.slug.clone()],
        volumes: vec![(row.volume.clone(), spec.data_dir.to_string())],
    };
    let started = state.runtime.run(&container).await?;

    if let Err(e) = wait_ready(state, &started.id, started.host_port).await {
        let _ = state.runtime.remove(&started.id).await;
        return Err(e);
    }
    Ok((started.id, started.host_port))
}

async fn wait_ready(state: &AppState, container_id: &str, host_port: u16) -> Result<()> {
    let begin = Instant::now();
    // The official images run a private, socket-only server while they
    // initialise, so the port only answers once the real server is up.
    let mut passes = 0;
    loop {
        match state.runtime.state(container_id).await? {
            ContainerState::Exited(code) => bail!(
                "the database exited{} while starting. {}",
                code.map(|c| format!(" with code {c}")).unwrap_or_default(),
                last_output(state, container_id).await
            ),
            ContainerState::Missing => bail!("the database container disappeared while starting"),
            ContainerState::Running | ContainerState::Other(_) => {}
        }
        passes = if deploy::tcp_ready(host_port).await {
            passes + 1
        } else {
            0
        };
        if passes >= 2 {
            return Ok(());
        }
        if begin.elapsed() > READY_TIMEOUT {
            bail!(
                "the database did not accept connections within {}s. {}",
                READY_TIMEOUT.as_secs(),
                last_output(state, container_id).await
            );
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// The last few lines the container printed, for the error message.
async fn last_output(state: &AppState, container_id: &str) -> String {
    use futures_util::StreamExt;
    let mut stream = state.runtime.logs(container_id, 5, false);
    let mut lines = Vec::new();
    while let Ok(Some(Ok(msg))) = tokio::time::timeout(Duration::from_secs(3), stream.next()).await
    {
        lines.push(msg.line);
    }
    if lines.is_empty() {
        "It printed nothing".to_string()
    } else {
        format!("Last output: {}", lines.join(" | "))
    }
}

/// Stop and remove the database's container. The volume is untouched.
pub async fn remove_containers(state: &AppState, id: &str) -> Result<()> {
    for container in state.runtime.list_by_label(LABEL_DATABASE, id).await? {
        state.runtime.stop(&container, STOP_GRACE_SECS).await?;
        state.runtime.remove(&container).await?;
    }
    Ok(())
}

/// Remove the container and the data volume. Irreversible.
pub async fn destroy(state: &AppState, id: &str, volume: &str) -> Result<()> {
    remove_containers(state, id).await?;
    state.runtime.remove_volume(volume).await
}

/// Remove every database of a project, then the project's network.
pub async fn destroy_project(state: &AppState, project_id: &str) -> Result<()> {
    let rows = sqlx::query("SELECT id, volume FROM databases WHERE project_id = ?")
        .bind(project_id)
        .fetch_all(&state.db)
        .await?;
    for r in rows {
        let id: String = r.get("id");
        destroy(state, &id, r.get("volume")).await?;
        crate::backups::remove_all(state, &id).await;
    }
    state
        .runtime
        .remove_network(&deploy::project_network(project_id))
        .await
}

/// Databases caught mid-start or mid-operation by a restart: adopt any
/// container that already finished starting, or fail the row if it never came up.
pub async fn recover(state: &AppState) -> Result<()> {
    let creating: Vec<String> =
        sqlx::query_scalar("SELECT id FROM databases WHERE status = 'creating'")
            .fetch_all(&state.db)
            .await?;
    for id in creating {
        let mut adopted = None;
        if let Ok(cids) = state.runtime.list_by_label(LABEL_DATABASE, &id).await {
            for cid in cids {
                if let Ok(Some(health)) = state.runtime.health(&cid).await
                    && health.state == ContainerState::Running
                    && let Some(port) = health.host_port
                    && wait_ready(state, &cid, port).await.is_ok()
                {
                    adopted = Some((cid, port));
                    break;
                }
                let _ = state.runtime.remove(&cid).await;
            }
        }
        if let Some((cid, port)) = adopted {
            sqlx::query(
                "UPDATE databases
                    SET status = 'running', error = NULL, container_id = ?, host_port = ?
                  WHERE id = ?",
            )
            .bind(&cid)
            .bind(i64::from(port))
            .bind(&id)
            .execute(&state.db)
            .await?;
        } else {
            sqlx::query(
                "UPDATE databases
                    SET status = 'failed',
                        error = 'Rig restarted while this database was starting. Start it again.'
                  WHERE id = ? AND status = 'creating'",
            )
            .bind(&id)
            .execute(&state.db)
            .await?;
        }
    }
    sqlx::query("UPDATE databases SET busy_op = NULL WHERE busy_op IS NOT NULL")
        .execute(&state.db)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_safe() {
        assert_eq!(
            identifiers(DatabaseEngine::Postgres, "main-db").0,
            "main_db"
        );
        assert_eq!(
            identifiers(DatabaseEngine::Mysql, "2024-archive").0,
            "db_2024_archive"
        );
        assert_eq!(
            identifiers(DatabaseEngine::Redis, "cache"),
            ("default".into(), "0".into())
        );
        let long = identifiers(DatabaseEngine::Postgres, &"a".repeat(48)).0;
        assert_eq!(long.len(), 32);
    }

    #[test]
    fn passwords_need_no_quoting() {
        let p = new_password();
        assert_eq!(p.len(), 48);
        assert!(p.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(p, new_password());
    }

    #[test]
    fn urls_and_images() {
        let pg = engine_spec(DatabaseEngine::Postgres);
        assert_eq!(
            pg.url("app", "s3", "main-db", 5432, "app"),
            "postgres://app:s3@main-db:5432/app"
        );
        assert_eq!(pg.image("17"), "postgres:17-alpine");
        let redis = engine_spec(DatabaseEngine::Redis);
        assert_eq!(
            redis.url("default", "s3", "cache", 6379, "0"),
            "redis://default:s3@cache:6379/0"
        );
        assert_eq!(
            redis.env("default", "s3", "0"),
            vec![("REDIS_PASSWORD".into(), "s3".into())]
        );
        // Password must never appear in the container command line.
        let cmd = redis.command().unwrap();
        assert!(cmd.iter().all(|a| !a.contains("s3")));
        assert!(cmd.iter().any(|a| a.contains("redis-server -")));
    }

    #[tokio::test]
    async fn recover_adopts_live_database_container_and_fails_missing_one() {
        use std::sync::Arc;

        // Bind a real loopback TCP listener that holds connections open so
        // `deploy::tcp_ready(port)` returns true for the adopted database.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let live_port = listener.local_addr().unwrap().port();
        let accept_task = tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((stream, _)) = listener.accept().await {
                held.push(stream);
            }
        });

        struct DbRecoverRuntime {
            live_port: u16,
        }

        #[async_trait::async_trait]
        impl crate::runtime::Runtime for DbRecoverRuntime {
            async fn info(&self) -> Result<crate::runtime::EngineInfo> {
                Ok(crate::runtime::EngineInfo {
                    name: "docker".into(),
                    version: "27.0".into(),
                })
            }
            async fn pull(&self, _: &str, _: &LogSink) -> Result<()> {
                Ok(())
            }
            async fn build(&self, _: crate::runtime::BuildSpec, _: &LogSink) -> Result<()> {
                Ok(())
            }
            async fn build_plan(&self, _: crate::runtime::PlanSpec, _: &LogSink) -> Result<()> {
                Ok(())
            }
            async fn run(&self, _: &ContainerSpec) -> Result<crate::runtime::Started> {
                bail!("unused")
            }
            async fn state(&self, id: &str) -> Result<ContainerState> {
                Ok(self
                    .health(id)
                    .await?
                    .map_or(ContainerState::Missing, |h| h.state))
            }
            async fn health(&self, id: &str) -> Result<Option<crate::runtime::Health>> {
                if id == "cid-db-live" {
                    Ok(Some(crate::runtime::Health {
                        state: ContainerState::Running,
                        restart_count: 0,
                        oom_killed: false,
                        exit_code: Some(0),
                        host_port: Some(self.live_port),
                    }))
                } else {
                    Ok(None)
                }
            }
            async fn start(&self, _: &str) -> Result<()> {
                Ok(())
            }
            async fn stop(&self, _: &str, _: u32) -> Result<()> {
                Ok(())
            }
            async fn remove(&self, _: &str) -> Result<()> {
                Ok(())
            }
            fn logs_with(
                &self,
                _: &str,
                _: crate::runtime::LogOptions,
            ) -> crate::runtime::LogStream {
                Box::pin(futures_util::stream::empty())
            }
            async fn list_by_label(&self, label: &str, value: &str) -> Result<Vec<String>> {
                if label == LABEL_DATABASE && value == "db-live" {
                    Ok(vec!["cid-db-live".into()])
                } else {
                    Ok(vec![])
                }
            }
            async fn remove_volume(&self, _: &str) -> Result<()> {
                Ok(())
            }
            async fn remove_network(&self, _: &str) -> Result<()> {
                Ok(())
            }
            async fn shell(
                &self,
                _: &str,
                _: u16,
                _: u16,
            ) -> Result<crate::runtime::Shell> {
                bail!("unused")
            }
            async fn image_id(&self, _: &str) -> Result<Option<String>> {
                Ok(None)
            }
            async fn usage(&self, _: &str) -> Result<Option<crate::runtime::Usage>> {
                Ok(None)
            }
            async fn resize_shell(&self, _: &str, _: u16, _: u16) -> Result<()> {
                Ok(())
            }
            async fn exec(
                &self,
                _: &str,
                _: Vec<String>,
                _: Vec<String>,
                _: Option<crate::runtime::ByteReader>,
                _: crate::runtime::ByteWriter,
            ) -> Result<crate::runtime::ExecOutcome> {
                bail!("unused")
            }
        }

        struct NoopProxy;
        #[async_trait::async_trait]
        impl crate::proxy::Proxy for NoopProxy {
            async fn ready(&self) -> bool {
                true
            }
            async fn apply(
                &self,
                _: &[crate::proxy::Route],
                _: Option<&rig_api::DashboardDomain>,
                _: &[String],
            ) -> Result<()> {
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
            }),
            runtime: Arc::new(DbRecoverRuntime { live_port }),
            proxy: Arc::new(NoopProxy),
            deploys: Arc::new(crate::deploy::Deployer::new().unwrap()),
            access: crate::access::DashboardAccess::default(),
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
            "INSERT INTO databases (id, org_id, project_id, name, slug, engine, version, volume, db_name, username, password_enc, status, created_at)
             VALUES ('db-live', 'org1', 'p1', 'Live', 'live', 'postgres', '17', 'rig-db-live', 'live', 'live', 'enc', 'creating', '2026-11-22T00:00:00Z'),
                    ('db-dead', 'org1', 'p1', 'Dead', 'dead', 'postgres', '17', 'rig-db-dead', 'dead', 'dead', 'enc', 'creating', '2026-11-22T00:00:00Z')",
        )
        .execute(&db)
        .await
        .unwrap();

        recover(&state).await.unwrap();
        accept_task.abort();

        let (live_status, live_cid, live_recorded_port): (String, Option<String>, Option<i64>) =
            sqlx::query_as(
                "SELECT status, container_id, host_port FROM databases WHERE id = 'db-live'",
            )
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(live_status, "running");
        assert_eq!(live_cid.as_deref(), Some("cid-db-live"));
        assert_eq!(live_recorded_port, Some(i64::from(live_port)));

        let dead_status: String =
            sqlx::query_scalar("SELECT status FROM databases WHERE id = 'db-dead'")
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!(dead_status, "failed");
    }
}
