//! Backups of managed databases: each engine's own dump tool runs inside the
//! database container and its output streams to a file under the data
//! directory. Nothing is interpolated into a shell: commands are argv lists
//! and passwords travel in the exec environment.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rig_api::DatabaseEngine;
use sqlx::Row;
use tokio::{
    fs,
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader, BufWriter},
};

use crate::{
    crypto::{BACKUP_CHUNK_SIZE, BACKUP_MAGIC, Vault},
    runtime::ExecOutcome,
    state::{AppState, now},
};

/// The commands that dump and restore one engine.
struct Tooling {
    dump: Vec<String>,
    /// `None` when the engine has no in-place restore yet.
    restore: Option<Vec<String>>,
    env: Vec<String>,
    extension: &'static str,
}

fn tooling(engine: DatabaseEngine, user: &str, password: &str, db: &str) -> Tooling {
    let argv = |parts: &[&str]| parts.iter().map(|p| (*p).to_owned()).collect::<Vec<_>>();
    match engine {
        // The official image trusts connections over the local socket.
        DatabaseEngine::Postgres => Tooling {
            dump: argv(&[
                "pg_dump",
                "--username",
                user,
                "--dbname",
                db,
                "--format=custom",
                "--no-owner",
            ]),
            restore: Some(argv(&[
                "pg_restore",
                "--username",
                user,
                "--dbname",
                db,
                "--clean",
                "--if-exists",
                "--no-owner",
                "--single-transaction",
            ])),
            env: Vec::new(),
            extension: "dump",
        },
        DatabaseEngine::Mysql | DatabaseEngine::Mariadb => {
            let (dump, client) = if engine == DatabaseEngine::Mariadb {
                ("mariadb-dump", "mariadb")
            } else {
                ("mysqldump", "mysql")
            };
            Tooling {
                dump: argv(&[
                    dump,
                    "--user",
                    user,
                    "--single-transaction",
                    "--routines",
                    "--triggers",
                    "--no-tablespaces",
                    db,
                ]),
                restore: Some(argv(&[client, "--user", user, db])),
                env: vec![format!("MYSQL_PWD={password}")],
                extension: "sql",
            }
        }
        // The mongo tools only take credentials as flags, so the URI travels
        // in the exec environment and a fixed script expands it. Nothing the
        // user controls is ever part of the script.
        DatabaseEngine::Mongodb => Tooling {
            dump: argv(&[
                "sh",
                "-c",
                r#"exec mongodump --uri "$RIG_MONGO_URI" --archive --quiet"#,
            ]),
            restore: Some(argv(&[
                "sh",
                "-c",
                r#"exec mongorestore --uri "$RIG_MONGO_URI" --archive --drop --quiet"#,
            ])),
            env: vec![format!(
                "RIG_MONGO_URI={}",
                crate::databases::engine_spec(DatabaseEngine::Mongodb).url(
                    user,
                    password,
                    "127.0.0.1",
                    27017,
                    db
                )
            )],
            extension: "archive",
        },
        DatabaseEngine::Redis => Tooling {
            dump: argv(&["redis-cli", "--rdb", "-"]),
            restore: None,
            env: vec![format!("REDISCLI_AUTH={password}")],
            extension: "rdb",
        },
    }
}

pub fn restorable(engine: DatabaseEngine) -> bool {
    engine != DatabaseEngine::Redis
}

fn dir(state: &AppState, database_id: &str) -> PathBuf {
    state.config.data_dir.join("backups").join(database_id)
}

/// The running container and decrypted credentials of a database.
struct Target {
    container_id: String,
    engine: DatabaseEngine,
    username: String,
    password: String,
    db_name: String,
}

async fn target(state: &AppState, database_id: &str) -> Result<Target> {
    let r = sqlx::query(
        "SELECT engine, status, container_id, username, password_enc, db_name
           FROM databases WHERE id = ?",
    )
    .bind(database_id)
    .fetch_one(&state.db)
    .await
    .context("loading the database")?;
    let status: String = r.get("status");
    let container_id: Option<String> = r.get("container_id");
    let (Some(container_id), "running") = (container_id, status.as_str()) else {
        bail!("the database is not running. Start it first");
    };
    let engine: String = r.get("engine");
    Ok(Target {
        container_id,
        engine: DatabaseEngine::parse(&engine).context("unknown engine")?,
        username: r.get("username"),
        password: state.vault.decrypt(&r.get::<String, _>("password_enc"))?,
        db_name: r.get("db_name"),
    })
}

fn failure(what: &str, outcome: &ExecOutcome) -> anyhow::Error {
    let code = outcome
        .exit_code
        .map_or_else(|| "no exit code".to_owned(), |c| format!("exit code {c}"));
    if outcome.stderr.is_empty() {
        anyhow::anyhow!("{what} failed ({code})")
    } else {
        anyhow::anyhow!("{what} failed ({code}): {}", outcome.stderr)
    }
}

/// Take the dump for a `running` backup row. Runs in its own task and records
/// the outcome on the row.
pub async fn run(state: AppState, backup_id: String, database_id: String) {
    let result = dump(&state, &backup_id, &database_id).await;
    let saved = match &result {
        Ok((path, size)) => {
            sqlx::query(
                "UPDATE backups SET status = 'ok', path = ?, size_bytes = ?, finished_at = ?
                  WHERE id = ?",
            )
            .bind(path.to_string_lossy().as_ref())
            .bind(*size)
            .bind(now())
            .bind(&backup_id)
            .execute(&state.db)
            .await
        }
        Err(e) => {
            tracing::warn!(backup = %backup_id, error = %format!("{e:#}"), "backup failed");
            let db: Option<(String, String)> =
                sqlx::query_as("SELECT org_id, name FROM databases WHERE id = ?")
                    .bind(&database_id)
                    .fetch_optional(&state.db)
                    .await
                    .ok()
                    .flatten();
            if let Some((org_id, name)) = db {
                crate::alerts::notify(
                    &state,
                    &org_id,
                    crate::alerts::Alert {
                        event: rig_api::AlertEvent::BackupFailed,
                        title: format!("Backing up {name} failed"),
                        detail: format!("{e:#}"),
                        target: Some(database_id.clone()),
                    },
                );
            }
            sqlx::query(
                "UPDATE backups SET status = 'failed', error = ?, finished_at = ? WHERE id = ?",
            )
            .bind(format!("{e:#}"))
            .bind(now())
            .bind(&backup_id)
            .execute(&state.db)
            .await
        }
    };
    if let Err(e) = saved {
        tracing::error!(backup = %backup_id, error = %e, "could not record backup state");
    }
}

/// Encrypt `reader` into `dest` using Rig's chunked XChaCha20-Poly1305 format
/// (`RIGBK1\0\0` || 16-byte stream base || sequence of `[u32 ct_len][u8 is_final][ct]`).
/// Returns total plaintext bytes written.
async fn write_encrypted_stream(
    vault: &Vault,
    mut reader: impl AsyncRead + Unpin,
    dest: fs::File,
) -> Result<u64> {
    let mut dest = BufWriter::with_capacity(256 * 1024, dest);
    let mut base = [0u8; 16];
    rand::fill(&mut base[..]);
    let mut header = [0u8; 24];
    header[..8].copy_from_slice(BACKUP_MAGIC);
    header[8..].copy_from_slice(&base);
    dest.write_all(&header).await?;

    let mut total: u64 = 0;
    let mut chunk_index: u64 = 0;
    let mut buf = vec![0u8; BACKUP_CHUNK_SIZE];
    let mut frame = Vec::with_capacity(5 + BACKUP_CHUNK_SIZE + 16);
    loop {
        let mut filled = 0;
        while filled < buf.len() {
            let n = reader.read(&mut buf[filled..]).await?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        let is_final = filled < buf.len();
        let ct = vault.encrypt_chunk(&base, chunk_index, is_final, &buf[..filled])?;
        let ct_len = u32::try_from(ct.len()).context("backup chunk too large")?;
        frame.clear();
        frame.extend_from_slice(&ct_len.to_le_bytes());
        frame.push(u8::from(is_final));
        frame.extend_from_slice(&ct);
        dest.write_all(&frame).await?;
        total = total.saturating_add(filled as u64);
        chunk_index += 1;
        if is_final {
            break;
        }
    }
    dest.flush().await?;
    dest.into_inner().sync_all().await?;
    Ok(total)
}

/// Opens a backup file for reading, transparently decrypting `RIGBK1\0\0`
/// chunked AEAD files while remaining backward-compatible with legacy plaintext dumps.
pub async fn open_decrypted(
    vault: Vault,
    path: &Path,
) -> Result<tokio::io::DuplexStream> {
    let file = fs::File::open(path).await.context("opening the backup")?;
    let mut file = BufReader::with_capacity(256 * 1024, file);
    let mut header = [0u8; 8];
    let n = file.read(&mut header).await.context("reading backup header")?;
    let (mut writer, reader) = tokio::io::duplex(BACKUP_CHUNK_SIZE * 2);
    if n == 8 && &header == BACKUP_MAGIC {
        let mut base = [0u8; 16];
        file.read_exact(&mut base)
            .await
            .context("reading backup stream nonce")?;
        tokio::spawn(async move {
            let mut chunk_index: u64 = 0;
            let mut ct = Vec::with_capacity(BACKUP_CHUNK_SIZE + 16);
            loop {
                let mut frame_hdr = [0u8; 5];
                if file.read_exact(&mut frame_hdr).await.is_err() {
                    // Truncated stream: drop writer early (incomplete)
                    return;
                }
                let ct_len = u32::from_le_bytes([
                    frame_hdr[0],
                    frame_hdr[1],
                    frame_hdr[2],
                    frame_hdr[3],
                ]) as usize;
                // Cap per-chunk ciphertext size (64 KiB + 16-byte Poly1305 tag).
                if ct_len > BACKUP_CHUNK_SIZE + 64 {
                    return;
                }
                let is_final = frame_hdr[4] != 0;
                ct.resize(ct_len, 0);
                if file.read_exact(&mut ct).await.is_err() {
                    return;
                }
                let Ok(pt) = vault.decrypt_chunk(&base, chunk_index, is_final, &ct) else {
                    tracing::error!("backup decryption failed at chunk {chunk_index}");
                    return;
                };
                if !pt.is_empty() && writer.write_all(&pt).await.is_err() {
                    return;
                }
                chunk_index += 1;
                if is_final {
                    let _ = writer.shutdown().await;
                    return;
                }
            }
        });
    } else {
        // Legacy unencrypted backup file: forward the bytes already read plus the rest.
        let prefix = header[..n].to_vec();
        tokio::spawn(async move {
            if !prefix.is_empty() && writer.write_all(&prefix).await.is_err() {
                return;
            }
            let _ = tokio::io::copy(&mut file, &mut writer).await;
            let _ = writer.shutdown().await;
        });
    }
    Ok(reader)
}

async fn dump(state: &AppState, backup_id: &str, database_id: &str) -> Result<(PathBuf, i64)> {
    let t = target(state, database_id).await?;
    let tools = tooling(t.engine, &t.username, &t.password, &t.db_name);
    let dir = dir(state, database_id);
    create_private_dir(&dir).await?;
    let path = dir.join(format!("{backup_id}.{}", tools.extension));
    let file = create_private_file(&path).await?;

    let (pipe_writer, pipe_reader) = tokio::io::duplex(BACKUP_CHUNK_SIZE * 2);
    let vault = state.vault.clone();
    let enc_task = tokio::spawn(async move { write_encrypted_stream(&vault, pipe_reader, file).await });

    let outcome = state
        .runtime
        .exec(&t.container_id, tools.dump, tools.env, None, Box::pin(pipe_writer))
        .await;
    let enc_result = enc_task.await.context("encrypting backup stream")?;
    let outcome = match outcome {
        Ok(o) if o.success() => o,
        Ok(o) => {
            fs::remove_file(&path).await.ok();
            return Err(failure("the dump", &o));
        }
        Err(e) => {
            fs::remove_file(&path).await.ok();
            return Err(e);
        }
    };
    drop(outcome);
    let plaintext_size = match enc_result {
        Ok(sz) => sz,
        Err(e) => {
            fs::remove_file(&path).await.ok();
            return Err(e);
        }
    };
    if plaintext_size == 0 {
        fs::remove_file(&path).await.ok();
        bail!("the dump came back empty");
    }
    Ok((path, i64::try_from(plaintext_size).unwrap_or(i64::MAX)))
}

/// Load a backup into its database, replacing what is there.
pub async fn restore(state: &AppState, database_id: &str, path: &Path) -> Result<()> {
    let t = target(state, database_id).await?;
    let tools = tooling(t.engine, &t.username, &t.password, &t.db_name);
    let Some(cmd) = tools.restore else {
        bail!("this engine cannot be restored in place yet");
    };
    let reader = open_decrypted(state.vault.clone(), path).await?;
    let outcome = state
        .runtime
        .exec(
            &t.container_id,
            cmd,
            tools.env,
            Some(Box::pin(reader)),
            Box::pin(tokio::io::sink()),
        )
        .await?;
    if !outcome.success() {
        return Err(failure("the restore", &outcome));
    }
    Ok(())
}

/// Forget every backup file of a database. The rows go with the database.
pub async fn remove_all(state: &AppState, database_id: &str) {
    let dir = dir(state, database_id);
    if let Err(e) = fs::remove_dir_all(&dir).await
        && e.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(dir = %dir.display(), error = %e, "could not remove backups");
    }
}

/// Backups caught mid-dump by a restart never finish on their own.
pub async fn recover(state: &AppState) -> Result<()> {
    sqlx::query(
        "UPDATE backups SET status = 'failed', finished_at = ?,
                error = 'Rig restarted during this backup. Take it again.'
          WHERE status = 'running'",
    )
    .bind(now())
    .execute(&state.db)
    .await?;
    Ok(())
}

async fn create_private_dir(dir: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    builder.mode(0o700);
    builder
        .create(dir)
        .await
        .with_context(|| format!("creating {}", dir.display()))
}

async fn create_private_file(path: &Path) -> Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    options
        .open(path)
        .await
        .with_context(|| format!("creating {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_stay_off_the_command_line() {
        for engine in [
            DatabaseEngine::Postgres,
            DatabaseEngine::Mysql,
            DatabaseEngine::Mariadb,
            DatabaseEngine::Redis,
            DatabaseEngine::Mongodb,
        ] {
            let t = tooling(engine, "app", "hunter2", "app");
            let argv = t.dump.iter().chain(t.restore.iter().flatten());
            assert!(
                argv.into_iter().all(|a| !a.contains("hunter2")),
                "{engine:?}"
            );
            assert_eq!(t.restore.is_some(), restorable(engine));
        }
    }

    #[tokio::test]
    async fn encrypted_backup_round_trip_and_tamper_detection() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::ephemeral();
        // Payload spanning multiple 64 KiB chunks.
        let mut plain = vec![0u8; BACKUP_CHUNK_SIZE * 2 + 1234];
        for (i, b) in plain.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        plain[100..112].copy_from_slice(b"SUPER_SECRET");

        let enc_path = dir.path().join("backup.dump");
        let file = create_private_file(&enc_path).await.unwrap();
        let written = write_encrypted_stream(&vault, plain.as_slice(), file)
            .await
            .unwrap();
        assert_eq!(written, plain.len() as u64);

        let on_disk = std::fs::read(&enc_path).unwrap();
        assert!(on_disk.starts_with(BACKUP_MAGIC));
        assert!(
            !on_disk.windows(12).any(|w| w == b"SUPER_SECRET"),
            "plaintext must not appear on disk"
        );

        let mut reader = open_decrypted(vault.clone(), &enc_path).await.unwrap();
        let mut recovered = Vec::new();
        reader.read_to_end(&mut recovered).await.unwrap();
        assert_eq!(recovered, plain);

        // Tampering with ciphertext causes decryption to fail (truncated/empty stream).
        let mut tampered = on_disk.clone();
        *tampered.last_mut().unwrap() ^= 0xff;
        let bad_path = dir.path().join("tampered.dump");
        std::fs::write(&bad_path, &tampered).unwrap();
        let mut reader = open_decrypted(vault.clone(), &bad_path).await.unwrap();
        let mut bad_out = Vec::new();
        reader.read_to_end(&mut bad_out).await.unwrap();
        assert_ne!(bad_out, plain);

        // Legacy unencrypted dumps remain readable.
        let legacy_path = dir.path().join("legacy.sql");
        std::fs::write(&legacy_path, b"CREATE TABLE t(x INT);").unwrap();
        let mut reader = open_decrypted(vault, &legacy_path).await.unwrap();
        let mut legacy_out = Vec::new();
        reader.read_to_end(&mut legacy_out).await.unwrap();
        assert_eq!(legacy_out, b"CREATE TABLE t(x INT);");
    }
}
