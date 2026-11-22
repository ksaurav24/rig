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

/// Databases caught mid-start or mid-operation by a restart will never finish on their own.
pub async fn recover(state: &AppState) -> Result<()> {
    sqlx::query(
        "UPDATE databases
            SET status = 'failed',
                error = 'Rig restarted while this database was starting. Start it again.'
          WHERE status = 'creating'",
    )
    .execute(&state.db)
    .await?;
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
}
