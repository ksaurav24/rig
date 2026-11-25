//! Portable full-server snapshot export, inspection, and cross-server restore.
//!
//! Snapshots are self-contained zstd-compressed tar archives (`manifest.json`,
//! `media/*`, `databases/*`), optionally wrapped in a passphrase-encrypted
//! `RIGSNAP1` AEAD envelope (Argon2id + XChaCha20-Poly1305).
//!
//! Because every Rig server has its own `master.key`, secrets (app env vars,
//! webhook secrets, database passwords, alert URLs/secrets, GitHub App keys)
//! are decrypted under the source server's `Vault` during export and freshly
//! re-encrypted under the target server's `Vault` during restore.

use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use argon2::Argon2;
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use rig_api::{
    Access, DatabaseEngine, SnapshotAppPreview, SnapshotDatabasePreview, SnapshotExportRequest,
    SnapshotInspectResult, SnapshotProjectPreview, SnapshotRestoreMode, SnapshotRestoreOptions,
    SnapshotRestoreReport, SourceKind, TlsMode,
};
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::{
    backups,
    crypto::random_token,
    databases, deploy, proxy,
    routes::org::{MEDIA_PREFIX, media_id_from_url},
    state::{AppState, new_id, now},
};

pub const SNAPSHOT_FORMAT_VERSION: u32 = 1;
pub const SNAPSHOT_ENC_MAGIC: &[u8; 8] = b"RIGSNAP1";
const ENC_FLAG_PASSPHRASE: u8 = 0x01;
const ENC_SALT_LEN: usize = 16;
const ENC_NONCE_LEN: usize = 24;
const ENC_HEADER_LEN: usize = 8 + 1 + ENC_SALT_LEN + ENC_NONCE_LEN;
const MAX_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotManifest {
    pub format_version: u32,
    pub created_at: String,
    pub rig_version: String,
    pub org: SnapshotOrg,
    #[serde(default)]
    pub media: Vec<SnapshotMedia>,
    #[serde(default)]
    pub projects: Vec<SnapshotProject>,
    #[serde(default)]
    pub apps: Vec<SnapshotApp>,
    #[serde(default)]
    pub databases: Vec<SnapshotDatabase>,
    #[serde(default)]
    pub alert_channels: Vec<SnapshotAlertChannel>,
    #[serde(default)]
    pub github_app: Option<SnapshotGithubApp>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotOrg {
    pub name: String,
    pub slug: String,
    pub logo_media_id: Option<String>,
    pub logo_external_url: Option<String>,
    pub accent_color: String,
    pub env_label: Option<String>,
    pub env_color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMedia {
    pub id: String,
    pub content_type: String,
    pub is_public: bool,
    pub archive_path: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotProject {
    pub name: String,
    pub slug: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotApp {
    pub project_slug: String,
    pub name: String,
    pub slug: String,
    pub source_kind: SourceKind,
    pub image: Option<String>,
    pub git_url: Option<String>,
    pub git_branch: Option<String>,
    pub dockerfile_path: String,
    pub port: u16,
    pub memory_mb: Option<u32>,
    pub cpu_millis: Option<u32>,
    pub health_path: Option<String>,
    pub auto_deploy: bool,
    pub was_running: bool,
    pub webhook_secret: String,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub entrypoint: Option<String>,
    #[serde(default)]
    pub extra_networks: Option<String>,
    #[serde(default)]
    pub volumes: Vec<SnapshotAppVolume>,
    #[serde(default)]
    pub domains: Vec<SnapshotDomain>,
    #[serde(default)]
    pub env: Vec<SnapshotEnvVar>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotAppVolume {
    pub mount_path: String,
    pub kind: String,
    pub read_only: bool,
    pub owned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotDomain {
    pub host: String,
    pub tls_mode: TlsMode,
    pub access: Access,
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotEnvVar {
    pub key: String,
    pub value: String,
    pub is_secret: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotDatabase {
    pub project_slug: String,
    pub name: String,
    pub slug: String,
    pub engine: DatabaseEngine,
    pub version: String,
    pub db_name: String,
    pub username: String,
    pub password: String,
    pub was_running: bool,
    pub dump_archive_path: Option<String>,
    pub dump_size_bytes: Option<u64>,
    pub dump_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotAlertChannel {
    pub name: String,
    pub url: String,
    pub url_host: String,
    pub signing_secret: String,
    pub events_raw: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotGithubApp {
    pub app_id: i64,
    pub slug: String,
    pub name: String,
    pub html_url: String,
    pub owner: String,
    pub client_id: String,
    pub client_secret: String,
    pub webhook_secret: String,
    pub private_key: String,
}

pub struct ExportedSnapshot {
    pub archive_path: PathBuf,
    pub filename: String,
    pub encrypted: bool,
    pub size_bytes: u64,
    pub _temp_dir: tempfile::TempDir,
}

fn derive_passphrase_key(passphrase: &str, salt: &[u8; ENC_SALT_LEN]) -> Result<[u8; 32]> {
    let mut key = [0u8; 32];
    Argon2::default()
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|e| anyhow!("deriving snapshot encryption key: {e}"))?;
    Ok(key)
}

pub fn encrypt_snapshot_bytes(plain: &[u8], passphrase: &str) -> Result<Vec<u8>> {
    let mut salt = [0u8; ENC_SALT_LEN];
    let mut nonce = [0u8; ENC_NONCE_LEN];
    rand::fill(&mut salt[..]);
    rand::fill(&mut nonce[..]);
    let key = derive_passphrase_key(passphrase, &salt)?;
    let cipher = XChaCha20Poly1305::new_from_slice(&key).map_err(|_| anyhow!("bad key"))?;
    let mut aad = Vec::with_capacity(9 + ENC_SALT_LEN);
    aad.extend_from_slice(SNAPSHOT_ENC_MAGIC);
    aad.push(ENC_FLAG_PASSPHRASE);
    aad.extend_from_slice(&salt);
    let ct = cipher
        .encrypt(
            &XNonce::from(nonce),
            Payload {
                msg: plain,
                aad: &aad,
            },
        )
        .map_err(|_| anyhow!("encrypting snapshot archive failed"))?;
    let mut out = Vec::with_capacity(ENC_HEADER_LEN + ct.len());
    out.extend_from_slice(SNAPSHOT_ENC_MAGIC);
    out.push(ENC_FLAG_PASSPHRASE);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

pub fn is_encrypted_snapshot(path: &Path) -> Result<bool> {
    let mut f = fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut magic = [0u8; 8];
    let n = f.read(&mut magic)?;
    Ok(n == 8 && &magic == SNAPSHOT_ENC_MAGIC)
}

fn decode_snapshot_to_tar_zst(
    src_path: &Path,
    work_dir: &Path,
    passphrase: Option<&str>,
) -> Result<(PathBuf, bool)> {
    let encrypted = is_encrypted_snapshot(src_path)?;
    if !encrypted {
        return Ok((src_path.to_path_buf(), false));
    }
    let pass = passphrase
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow!("this snapshot is passphrase-encrypted; enter the passphrase to open it")
        })?;
    let raw = fs::read(src_path).with_context(|| format!("reading {}", src_path.display()))?;
    if raw.len() <= ENC_HEADER_LEN {
        bail!("snapshot file is truncated");
    }
    if raw[8] != ENC_FLAG_PASSPHRASE {
        bail!("unsupported snapshot encryption format");
    }
    let salt: [u8; ENC_SALT_LEN] = raw[9..9 + ENC_SALT_LEN].try_into().expect("salt len");
    let nonce: [u8; ENC_NONCE_LEN] = raw[9 + ENC_SALT_LEN..ENC_HEADER_LEN]
        .try_into()
        .expect("nonce len");
    let ct = &raw[ENC_HEADER_LEN..];
    let key = derive_passphrase_key(pass, &salt)?;
    let cipher = XChaCha20Poly1305::new_from_slice(&key).map_err(|_| anyhow!("bad key"))?;
    let mut aad = Vec::with_capacity(9 + ENC_SALT_LEN);
    aad.extend_from_slice(SNAPSHOT_ENC_MAGIC);
    aad.push(ENC_FLAG_PASSPHRASE);
    aad.extend_from_slice(&salt);
    let plain = cipher
        .decrypt(
            &XNonce::from(nonce),
            Payload {
                msg: ct,
                aad: &aad,
            },
        )
        .map_err(|_| anyhow!("wrong passphrase or corrupted snapshot file"))?;
    let dec_path = work_dir.join("decrypted.tar.zst");
    fs::write(&dec_path, plain)?;
    Ok((dec_path, true))
}

fn safe_archive_entry_path(raw: &Path) -> Result<PathBuf> {
    let mut clean = PathBuf::new();
    for comp in raw.components() {
        match comp {
            std::path::Component::Normal(p) => clean.push(p),
            std::path::Component::CurDir => {}
            _ => bail!("invalid path in snapshot archive: {}", raw.display()),
        }
    }
    if clean.as_os_str().is_empty() {
        bail!("empty path in snapshot archive");
    }
    Ok(clean)
}

fn extract_snapshot_archive(
    src_path: &Path,
    extract_dir: &Path,
    passphrase: Option<&str>,
) -> Result<(SnapshotManifest, bool)> {
    let (tar_zst_path, encrypted) = decode_snapshot_to_tar_zst(src_path, extract_dir, passphrase)?;
    let file = fs::File::open(&tar_zst_path)
        .with_context(|| format!("opening {}", tar_zst_path.display()))?;
    let decoder = zstd::Decoder::new(file).context("snapshot is not a valid zstd archive")?;
    let mut archive = tar::Archive::new(decoder);

    let unpacked = extract_dir.join("unpacked");
    fs::create_dir_all(&unpacked)?;

    for entry in archive.entries().context("reading snapshot tar entries")? {
        let mut entry = entry.context("reading snapshot tar entry")?;
        let rel = safe_archive_entry_path(&entry.path().context("reading entry path")?)?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let dest = unpacked.join(&rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        entry.unpack(&dest)?;
    }

    let manifest_path = unpacked.join("manifest.json");
    let meta = fs::metadata(&manifest_path)
        .context("snapshot archive is missing manifest.json")?;
    if meta.len() > MAX_MANIFEST_BYTES {
        bail!("snapshot manifest exceeds size limit");
    }
    let manifest_raw = fs::read_to_string(&manifest_path).context("reading manifest.json")?;
    let manifest: SnapshotManifest =
        serde_json::from_str(&manifest_raw).context("parsing snapshot manifest.json")?;
    if manifest.format_version > SNAPSHOT_FORMAT_VERSION {
        bail!(
            "snapshot format v{} is newer than this Rig server supports (v{SNAPSHOT_FORMAT_VERSION})",
            manifest.format_version
        );
    }
    Ok((manifest, encrypted))
}

/// Export a complete, portable snapshot archive of `org_id`.
pub async fn export_snapshot(
    state: &AppState,
    org_id: &str,
    req: &SnapshotExportRequest,
) -> Result<ExportedSnapshot> {
    let tmp_dir = tempfile::tempdir().context("creating snapshot staging directory")?;
    let dumps_dir = tmp_dir.path().join("dumps");
    fs::create_dir_all(&dumps_dir)?;

    let org_row = sqlx::query(
        "SELECT name, slug, logo_url, accent_color, env_label, env_color
           FROM organizations WHERE id = ?",
    )
    .bind(org_id)
    .fetch_one(&state.db)
    .await
    .context("loading organization")?;
    let org_slug: String = org_row.get("slug");
    let logo_url: Option<String> = org_row.get("logo_url");
    let logo_media_id = logo_url.as_deref().and_then(media_id_from_url).map(str::to_owned);
    let logo_external_url = logo_url
        .as_deref()
        .filter(|u| !u.starts_with(MEDIA_PREFIX))
        .map(str::to_owned);

    let org = SnapshotOrg {
        name: org_row.get("name"),
        slug: org_slug.clone(),
        logo_media_id,
        logo_external_url,
        accent_color: org_row.get("accent_color"),
        env_label: org_row.get("env_label"),
        env_color: org_row.get("env_color"),
    };

    // Media blobs (org logos, uploaded images)
    let media_rows = sqlx::query(
        "SELECT id, content_type, data, is_public FROM media WHERE org_id = ? ORDER BY created_at",
    )
    .bind(org_id)
    .fetch_all(&state.db)
    .await?;
    let mut media_entries = Vec::with_capacity(media_rows.len());
    let mut media_blobs: Vec<(String, Vec<u8>)> = Vec::with_capacity(media_rows.len());
    for r in media_rows {
        let id: String = r.get("id");
        let content_type: String = r.get("content_type");
        let data: Vec<u8> = r.get("data");
        let is_public = r.get::<i64, _>("is_public") != 0;
        let archive_path = format!("media/{id}");
        media_entries.push(SnapshotMedia {
            id,
            content_type,
            is_public,
            archive_path: archive_path.clone(),
            size_bytes: data.len() as u64,
        });
        media_blobs.push((archive_path, data));
    }

    // Projects
    let project_rows = sqlx::query(
        "SELECT id, name, slug, description FROM projects WHERE org_id = ? ORDER BY created_at",
    )
    .bind(org_id)
    .fetch_all(&state.db)
    .await?;
    let mut project_slug_by_id: HashMap<String, String> = HashMap::new();
    let mut projects = Vec::with_capacity(project_rows.len());
    for r in project_rows {
        let id: String = r.get("id");
        let slug: String = r.get("slug");
        project_slug_by_id.insert(id, slug.clone());
        projects.push(SnapshotProject {
            name: r.get("name"),
            slug,
            description: r.get("description"),
        });
    }

    // Apps + env vars + domains + volumes
    let app_rows = sqlx::query(
        "SELECT id, project_id, name, slug, source_kind, image, git_url, git_branch,
                dockerfile_path, port, memory_mb, cpu_millis, health_path, status,
                webhook_secret, auto_deploy, command, entrypoint, extra_networks
           FROM apps WHERE org_id = ? ORDER BY created_at",
    )
    .bind(org_id)
    .fetch_all(&state.db)
    .await?;

    let mut apps = Vec::with_capacity(app_rows.len());
    for r in app_rows {
        let app_id: String = r.get("id");
        let project_id: String = r.get("project_id");
        let Some(project_slug) = project_slug_by_id.get(&project_id).cloned() else {
            continue;
        };
        let source_kind =
            SourceKind::parse(r.get::<&str, _>("source_kind")).unwrap_or(SourceKind::Image);
        let webhook_raw: String = r.get("webhook_secret");
        let webhook_secret = state
            .vault
            .decrypt(&webhook_raw)
            .unwrap_or(webhook_raw);
        let status: String = r.get("status");

        let vol_rows = sqlx::query(
            "SELECT mount_path, kind, read_only, owned FROM app_volumes WHERE app_id = ? ORDER BY mount_path",
        )
        .bind(&app_id)
        .fetch_all(&state.db)
        .await?;
        let volumes = vol_rows
            .into_iter()
            .map(|vr| SnapshotAppVolume {
                mount_path: vr.get("mount_path"),
                kind: vr.get("kind"),
                read_only: vr.get::<i64, _>("read_only") != 0,
                owned: vr.get::<i64, _>("owned") != 0,
            })
            .collect();

        let dom_rows = sqlx::query(
            "SELECT host, tls_mode, access, port FROM domains WHERE app_id = ? ORDER BY host",
        )
        .bind(&app_id)
        .fetch_all(&state.db)
        .await?;
        let domains = dom_rows
            .into_iter()
            .map(|dr| SnapshotDomain {
                host: dr.get("host"),
                tls_mode: TlsMode::parse(dr.get("tls_mode")).unwrap_or(TlsMode::Auto),
                access: Access::parse(dr.get("access")).unwrap_or(Access::Public),
                port: dr
                    .get::<Option<i64>, _>("port")
                    .and_then(|p| u16::try_from(p).ok()),
            })
            .collect();

        let env_rows = sqlx::query(
            "SELECT key, value_enc, is_secret FROM env_vars WHERE app_id = ? ORDER BY key",
        )
        .bind(&app_id)
        .fetch_all(&state.db)
        .await?;
        let mut env = Vec::with_capacity(env_rows.len());
        for er in env_rows {
            let enc: String = er.get("value_enc");
            let val = state
                .vault
                .decrypt(&enc)
                .with_context(|| format!("decrypting env var {}", er.get::<String, _>("key")))?;
            env.push(SnapshotEnvVar {
                key: er.get("key"),
                value: val,
                is_secret: er.get::<i64, _>("is_secret") != 0,
            });
        }

        apps.push(SnapshotApp {
            project_slug,
            name: r.get("name"),
            slug: r.get("slug"),
            source_kind,
            image: r.get("image"),
            git_url: r.get("git_url"),
            git_branch: r.get("git_branch"),
            dockerfile_path: r.get("dockerfile_path"),
            port: u16::try_from(r.get::<i64, _>("port")).unwrap_or(80),
            memory_mb: r
                .get::<Option<i64>, _>("memory_mb")
                .and_then(|v| u32::try_from(v).ok()),
            cpu_millis: r
                .get::<Option<i64>, _>("cpu_millis")
                .and_then(|v| u32::try_from(v).ok()),
            health_path: r.get("health_path"),
            auto_deploy: r.get::<i64, _>("auto_deploy") != 0,
            was_running: status == "running",
            webhook_secret,
            command: r.get("command"),
            entrypoint: r.get("entrypoint"),
            extra_networks: r.get("extra_networks"),
            volumes,
            domains,
            env,
        });
    }

    // Databases + live dumps (or latest ok backup fallback if stopped)
    let db_rows = sqlx::query(
        "SELECT id, project_id, name, slug, engine, version, db_name, username, password_enc, status
           FROM databases WHERE org_id = ? ORDER BY created_at",
    )
    .bind(org_id)
    .fetch_all(&state.db)
    .await?;

    let mut dbs = Vec::with_capacity(db_rows.len());
    let mut dump_files: Vec<(String, PathBuf)> = Vec::new();
    for r in db_rows {
        let db_id: String = r.get("id");
        let project_id: String = r.get("project_id");
        let Some(project_slug) = project_slug_by_id.get(&project_id).cloned() else {
            continue;
        };
        let slug: String = r.get("slug");
        let engine = DatabaseEngine::parse(r.get("engine"))
            .ok_or_else(|| anyhow!("unknown database engine"))?;
        let password = state
            .vault
            .decrypt(&r.get::<String, _>("password_enc"))
            .context("decrypting database password")?;
        let status: String = r.get("status");
        let was_running = status == "running";

        let mut dump_archive_path = None;
        let mut dump_size_bytes = None;
        let mut dump_error = None;

        if req.include_database_dumps {
            let stem = format!("{project_slug}__{slug}");
            if was_running {
                match backups::dump_plaintext_to_file(state, &db_id, &dumps_dir, &stem).await {
                    Ok((path, ext, sz)) => {
                        let arch = format!("databases/{stem}.{ext}");
                        dump_archive_path = Some(arch.clone());
                        dump_size_bytes = Some(sz);
                        dump_files.push((arch, path));
                    }
                    Err(e) => {
                        dump_error = Some(format!("{e:#}"));
                    }
                }
            } else {
                // If stopped, try reusing the latest successful on-disk backup.
                let latest_backup: Option<String> = sqlx::query_scalar(
                    "SELECT path FROM backups WHERE database_id = ? AND status = 'ok' AND path IS NOT NULL
                      ORDER BY created_at DESC LIMIT 1",
                )
                .bind(&db_id)
                .fetch_optional(&state.db)
                .await?
                .flatten();
                if let Some(bk_path) = latest_backup {
                    let bk_p = Path::new(&bk_path);
                    if bk_p.exists() {
                        let ext = bk_p.extension().and_then(|e| e.to_str()).unwrap_or("dump");
                        let out_p = dumps_dir.join(format!("{stem}.{ext}"));
                        match backups::open_decrypted(state.vault.clone(), bk_p).await {
                            Ok(mut reader) => {
                                let mut out_f = tokio::fs::File::create(&out_p).await?;
                                match tokio::io::copy(&mut reader, &mut out_f).await {
                                    Ok(sz) if sz > 0 => {
                                        let arch = format!("databases/{stem}.{ext}");
                                        dump_archive_path = Some(arch.clone());
                                        dump_size_bytes = Some(sz);
                                        dump_files.push((arch, out_p));
                                    }
                                    Ok(_) => {
                                        let _ = tokio::fs::remove_file(&out_p).await;
                                        dump_error = Some(
                                            "Database was stopped and latest backup was empty".into(),
                                        );
                                    }
                                    Err(e) => {
                                        let _ = tokio::fs::remove_file(&out_p).await;
                                        dump_error = Some(format!("decrypting latest backup: {e}"));
                                    }
                                }
                            }
                            Err(e) => {
                                dump_error = Some(format!("opening latest backup: {e:#}"));
                            }
                        }
                    } else {
                        dump_error = Some("Database is stopped and has no saved backup".into());
                    }
                } else {
                    dump_error = Some("Database is stopped; start it to include a live dump".into());
                }
            }
        }

        dbs.push(SnapshotDatabase {
            project_slug,
            name: r.get("name"),
            slug,
            engine,
            version: r.get("version"),
            db_name: r.get("db_name"),
            username: r.get("username"),
            password,
            was_running,
            dump_archive_path,
            dump_size_bytes,
            dump_error,
        });
    }

    // Alert channels
    let mut alert_channels = Vec::new();
    if req.include_alerts {
        let rows = sqlx::query(
            "SELECT name, url_enc, url_host, signing_secret, events, enabled
               FROM alert_channels WHERE org_id = ? ORDER BY created_at",
        )
        .bind(org_id)
        .fetch_all(&state.db)
        .await?;
        for r in rows {
            let url = state.vault.decrypt(&r.get::<String, _>("url_enc"))?;
            let sig_raw: String = r.get("signing_secret");
            let signing_secret = state.vault.decrypt(&sig_raw).unwrap_or(sig_raw);
            alert_channels.push(SnapshotAlertChannel {
                name: r.get("name"),
                url,
                url_host: r.get("url_host"),
                signing_secret,
                events_raw: r.get("events"),
                enabled: r.get::<i64, _>("enabled") != 0,
            });
        }
    }

    // Optional GitHub App
    let github_app = if req.include_github {
        sqlx::query(
            "SELECT app_id, slug, name, html_url, owner, client_id, client_secret, webhook_secret, private_key
               FROM github_apps WHERE org_id = ?",
        )
        .bind(org_id)
        .fetch_optional(&state.db)
        .await?
        .map(|r| -> Result<SnapshotGithubApp> {
            Ok(SnapshotGithubApp {
                app_id: r.get("app_id"),
                slug: r.get("slug"),
                name: r.get("name"),
                html_url: r.get("html_url"),
                owner: r.get("owner"),
                client_id: r.get("client_id"),
                client_secret: state.vault.decrypt(&r.get::<String, _>("client_secret"))?,
                webhook_secret: state.vault.decrypt(&r.get::<String, _>("webhook_secret"))?,
                private_key: state.vault.decrypt(&r.get::<String, _>("private_key"))?,
            })
        })
        .transpose()?
    } else {
        None
    };

    let manifest = SnapshotManifest {
        format_version: SNAPSHOT_FORMAT_VERSION,
        created_at: now(),
        rig_version: env!("CARGO_PKG_VERSION").to_owned(),
        org,
        media: media_entries,
        projects,
        apps,
        databases: dbs,
        alert_channels,
        github_app,
    };

    let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    let passphrase = req
        .passphrase
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let encrypted = passphrase.is_some();

    let tar_zst_path = tmp_dir.path().join("snapshot.tar.zst");
    let final_path = tmp_dir.path().join(if encrypted {
        "snapshot.rig-snapshot.enc"
    } else {
        "snapshot.rig-snapshot.tar.zst"
    });
    let tar_zst_clone = tar_zst_path.clone();
    let final_clone = final_path.clone();

    tokio::task::spawn_blocking(move || -> Result<()> {
        let out_file = fs::File::create(&tar_zst_clone)?;
        let encoder = zstd::Encoder::new(out_file, 3)?;
        let mut tar = tar::Builder::new(encoder);

        let mut hdr = tar::Header::new_gnu();
        hdr.set_size(manifest_bytes.len() as u64);
        hdr.set_mode(0o600);
        hdr.set_cksum();
        tar.append_data(&mut hdr, "manifest.json", manifest_bytes.as_slice())?;

        for (arch_path, data) in media_blobs {
            let mut mhdr = tar::Header::new_gnu();
            mhdr.set_size(data.len() as u64);
            mhdr.set_mode(0o600);
            mhdr.set_cksum();
            tar.append_data(&mut mhdr, arch_path, data.as_slice())?;
        }

        for (arch_path, disk_path) in dump_files {
            tar.append_path_with_name(&disk_path, &arch_path)?;
        }

        let encoder = tar.into_inner()?;
        let mut out_file = encoder.finish()?;
        out_file.flush()?;
        drop(out_file);

        if let Some(pass) = passphrase {
            let raw = fs::read(&tar_zst_clone)?;
            let enc = encrypt_snapshot_bytes(&raw, &pass)?;
            fs::write(&final_clone, enc)?;
            let _ = fs::remove_file(&tar_zst_clone);
        } else {
            fs::rename(&tar_zst_clone, &final_clone)?;
        }
        Ok(())
    })
    .await
    .map_err(|e| anyhow!("snapshot packaging task failed: {e}"))??;

    let size_bytes = fs::metadata(&final_path)?.len();
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let ext = if encrypted {
        "rig-snapshot.enc"
    } else {
        "rig-snapshot.tar.zst"
    };
    let filename = format!("{org_slug}-snapshot-{stamp}.{ext}");

    Ok(ExportedSnapshot {
        archive_path: final_path,
        filename,
        encrypted,
        size_bytes,
        _temp_dir: tmp_dir,
    })
}

/// Inspect a snapshot archive on disk and compare against `org_id`'s current state.
pub async fn inspect_snapshot(
    state: &AppState,
    org_id: &str,
    archive_path: &Path,
    passphrase: Option<&str>,
) -> Result<SnapshotInspectResult> {
    let work_dir = tempfile::tempdir()?;
    let src = archive_path.to_path_buf();
    let work = work_dir.path().to_path_buf();
    let pass = passphrase.map(str::to_owned);
    let (manifest, encrypted) = tokio::task::spawn_blocking(move || {
        extract_snapshot_archive(&src, &work, pass.as_deref())
    })
    .await
    .map_err(|e| anyhow!("snapshot inspect task failed: {e}"))??;

    let existing_projects: Vec<(String, String)> =
        sqlx::query_as("SELECT id, slug FROM projects WHERE org_id = ?")
            .bind(org_id)
            .fetch_all(&state.db)
            .await?;
    let project_id_by_slug: HashMap<String, String> = existing_projects
        .into_iter()
        .map(|(id, slug)| (slug, id))
        .collect();

    let existing_apps: Vec<(String, String)> = sqlx::query_as(
        "SELECT p.slug, a.slug FROM apps a JOIN projects p ON p.id = a.project_id WHERE a.org_id = ?",
    )
    .bind(org_id)
    .fetch_all(&state.db)
    .await?;

    let existing_dbs: Vec<(String, String)> = sqlx::query_as(
        "SELECT p.slug, d.slug FROM databases d JOIN projects p ON p.id = d.project_id WHERE d.org_id = ?",
    )
    .bind(org_id)
    .fetch_all(&state.db)
    .await?;

    let mut warnings = Vec::new();

    let projects = manifest
        .projects
        .iter()
        .map(|p| SnapshotProjectPreview {
            name: p.name.clone(),
            slug: p.slug.clone(),
            description: p.description.clone(),
            exists_on_target: project_id_by_slug.contains_key(&p.slug),
        })
        .collect();

    let apps = manifest
        .apps
        .iter()
        .map(|a| {
            if a.source_kind == SourceKind::Local {
                warnings.push(format!(
                    "App `{}` uses local source deploys; config, env vars, and domains will be restored, then re-push code via `rig up` or Local Git.",
                    a.name
                ));
            }
            SnapshotAppPreview {
                project_slug: a.project_slug.clone(),
                name: a.name.clone(),
                slug: a.slug.clone(),
                source_kind: a.source_kind,
                image: a.image.clone(),
                git_url: a.git_url.clone(),
                port: a.port,
                was_running: a.was_running,
                env_count: a.env.len(),
                domain_count: a.domains.len(),
                volume_count: a.volumes.len(),
                exists_on_target: existing_apps
                    .iter()
                    .any(|(ps, s)| ps == &a.project_slug && s == &a.slug),
            }
        })
        .collect();

    let databases = manifest
        .databases
        .iter()
        .map(|d| {
            if d.engine == DatabaseEngine::Redis && d.dump_archive_path.is_some() {
                warnings.push(format!(
                    "Redis database `{}` includes an RDB snapshot file, but Redis in-place restore is not supported yet; credentials & container will be recreated.",
                    d.name
                ));
            }
            if let Some(err) = &d.dump_error {
                warnings.push(format!("Database `{}` dump note: {err}", d.name));
            }
            SnapshotDatabasePreview {
                project_slug: d.project_slug.clone(),
                name: d.name.clone(),
                slug: d.slug.clone(),
                engine: d.engine,
                version: d.version.clone(),
                was_running: d.was_running,
                has_dump: d.dump_archive_path.is_some(),
                dump_size_bytes: d.dump_size_bytes,
                dump_error: d.dump_error.clone(),
                exists_on_target: existing_dbs
                    .iter()
                    .any(|(ps, s)| ps == &d.project_slug && s == &d.slug),
            }
        })
        .collect();

    Ok(SnapshotInspectResult {
        format_version: manifest.format_version,
        encrypted,
        created_at: manifest.created_at,
        rig_version: manifest.rig_version,
        source_org_name: manifest.org.name,
        source_org_slug: manifest.org.slug,
        has_logo: manifest.org.logo_media_id.is_some() || manifest.org.logo_external_url.is_some(),
        has_github_app: manifest.github_app.is_some(),
        media_count: manifest.media.len(),
        alert_channel_count: manifest.alert_channels.len(),
        projects,
        apps,
        databases,
        warnings,
    })
}

/// Restore a portable snapshot archive onto `org_id` on this server.
pub async fn restore_snapshot(
    state: &AppState,
    org_id: &str,
    actor_user_id: &str,
    archive_path: &Path,
    passphrase: Option<&str>,
    opts: &SnapshotRestoreOptions,
) -> Result<SnapshotRestoreReport> {
    let work_dir = tempfile::tempdir()?;
    let src = archive_path.to_path_buf();
    let work = work_dir.path().to_path_buf();
    let pass = passphrase.map(str::to_owned);
    let (manifest, _encrypted) = tokio::task::spawn_blocking(move || {
        extract_snapshot_archive(&src, &work, pass.as_deref())
    })
    .await
    .map_err(|e| anyhow!("snapshot unpack task failed: {e}"))??;

    let unpacked_dir = work_dir.path().join("unpacked");
    let mut warnings: Vec<String> = Vec::new();

    // If Replace mode: cleanly tear down existing org projects, apps, databases, and alert channels,
    // while ALWAYS keeping users, memberships, sessions, and paired device tokens intact!
    if opts.mode == SnapshotRestoreMode::Replace {
        let app_ids: Vec<String> = sqlx::query_scalar("SELECT id FROM apps WHERE org_id = ?")
            .bind(org_id)
            .fetch_all(&state.db)
            .await?;
        for aid in &app_ids {
            let _ = deploy::remove_app_containers(state, aid).await;
            let _ = deploy::remove_app_volumes(state, aid).await;
        }
        let proj_ids: Vec<String> = sqlx::query_scalar("SELECT id FROM projects WHERE org_id = ?")
            .bind(org_id)
            .fetch_all(&state.db)
            .await?;
        for pid in &proj_ids {
            let _ = databases::destroy_project(state, pid).await;
        }
        sqlx::query("DELETE FROM projects WHERE org_id = ?")
            .bind(org_id)
            .execute(&state.db)
            .await?;
        if opts.restore_alerts {
            sqlx::query("DELETE FROM alert_channels WHERE org_id = ?")
                .bind(org_id)
                .execute(&state.db)
                .await?;
        }
    }

    let at = now();
    let server_id: Option<String> =
        sqlx::query_scalar("SELECT id FROM servers WHERE org_id = ? AND kind = 'local' LIMIT 1")
            .bind(org_id)
            .fetch_optional(&state.db)
            .await?;

    // 1. Media blobs (map old media ID -> new media ID)
    let mut media_id_map: HashMap<String, String> = HashMap::new();
    let mut media_restored = 0usize;
    if opts.restore_branding {
        for m in &manifest.media {
            let rel = safe_archive_entry_path(Path::new(&m.archive_path))?;
            let disk = unpacked_dir.join(rel);
            if let Ok(bytes) = fs::read(&disk) {
                let nid = new_id();
                sqlx::query(
                    "INSERT INTO media (id, org_id, content_type, data, is_public, created_at)
                     VALUES (?, ?, ?, ?, ?, ?)",
                )
                .bind(&nid)
                .bind(org_id)
                .bind(&m.content_type)
                .bind(&bytes)
                .bind(if m.is_public { 1i64 } else { 0i64 })
                .bind(&at)
                .execute(&state.db)
                .await?;
                media_id_map.insert(m.id.clone(), nid);
                media_restored += 1;
            }
        }
    }

    // 2. Organization branding
    let mut branding_restored = false;
    if opts.restore_branding {
        let next_logo = if let Some(old_mid) = &manifest.org.logo_media_id {
            media_id_map
                .get(old_mid)
                .map(|nid| format!("{MEDIA_PREFIX}{nid}"))
        } else {
            manifest.org.logo_external_url.clone()
        };
        sqlx::query(
            "UPDATE organizations
                SET name = ?, logo_url = ?, accent_color = ?, env_label = ?, env_color = ?
              WHERE id = ?",
        )
        .bind(&manifest.org.name)
        .bind(&next_logo)
        .bind(&manifest.org.accent_color)
        .bind(&manifest.org.env_label)
        .bind(&manifest.org.env_color)
        .bind(org_id)
        .execute(&state.db)
        .await?;
        branding_restored = true;
    }

    // 3. Projects
    let mut project_id_by_slug: HashMap<String, String> = HashMap::new();
    let existing_projs: Vec<(String, String)> =
        sqlx::query_as("SELECT id, slug FROM projects WHERE org_id = ?")
            .bind(org_id)
            .fetch_all(&state.db)
            .await?;
    for (id, slug) in existing_projs {
        project_id_by_slug.insert(slug, id);
    }

    let mut projects_created = 0usize;
    let mut projects_skipped = 0usize;
    for p in &manifest.projects {
        if project_id_by_slug.contains_key(&p.slug) {
            projects_skipped += 1;
            continue;
        }
        let pid = new_id();
        sqlx::query(
            "INSERT INTO projects (id, org_id, name, slug, description, created_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&pid)
        .bind(org_id)
        .bind(&p.name)
        .bind(&p.slug)
        .bind(&p.description)
        .bind(&at)
        .execute(&state.db)
        .await?;
        project_id_by_slug.insert(p.slug.clone(), pid);
        projects_created += 1;
    }

    // 4. Databases (created BEFORE apps so databases start initializing right away)
    let mut databases_created = 0usize;
    let mut databases_skipped = 0usize;
    let mut databases_provisioning = 0usize;
    let mut database_dumps_scheduled = 0usize;
    let mut pending_db_restores: Vec<(String, Option<PathBuf>)> = Vec::new();

    for d in &manifest.databases {
        let Some(project_id) = project_id_by_slug.get(&d.project_slug).cloned() else {
            warnings.push(format!(
                "Skipped database `{}`: project `{}` was not found",
                d.name, d.project_slug
            ));
            databases_skipped += 1;
            continue;
        };
        let clash: i64 = sqlx::query_scalar(
            "SELECT (SELECT COUNT(*) FROM databases WHERE project_id = ?1 AND (slug = ?2 OR name = ?3))
                  + (SELECT COUNT(*) FROM apps WHERE project_id = ?1 AND slug = ?2)",
        )
        .bind(&project_id)
        .bind(&d.slug)
        .bind(&d.name)
        .fetch_one(&state.db)
        .await?;
        if clash > 0 {
            databases_skipped += 1;
            warnings.push(format!(
                "Skipped existing database `{}/{}` (use Replace mode to overwrite)",
                d.project_slug, d.name
            ));
            continue;
        }

        let db_id = new_id();
        let password_enc = state.vault.encrypt(&d.password)?;
        let initial_status = if opts.start_databases && d.was_running {
            "creating"
        } else {
            "stopped"
        };
        sqlx::query(
            "INSERT INTO databases (id, org_id, project_id, name, slug, engine, version, volume,
                                    db_name, username, password_enc, status, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&db_id)
        .bind(org_id)
        .bind(&project_id)
        .bind(&d.name)
        .bind(&d.slug)
        .bind(d.engine.as_str())
        .bind(&d.version)
        .bind(databases::volume_name(&db_id))
        .bind(&d.db_name)
        .bind(&d.username)
        .bind(&password_enc)
        .bind(initial_status)
        .bind(&at)
        .execute(&state.db)
        .await?;
        databases_created += 1;

        if opts.start_databases && d.was_running {
            databases_provisioning += 1;
            let persisted_dump = if let Some(arch) = &d.dump_archive_path {
                if backups::restorable(d.engine) {
                    let rel = safe_archive_entry_path(Path::new(arch))?;
                    let src_dump = unpacked_dir.join(rel);
                    if src_dump.exists() {
                        // Persist the dump outside `work_dir` so the async restore task can read it after this handler returns.
                        let staging = state
                            .config
                            .data_dir
                            .join("backups")
                            .join("_snapshot_restore");
                        let _ = fs::create_dir_all(&staging);
                        let ext = src_dump
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("dump");
                        let dest = staging.join(format!("{db_id}.{ext}"));
                        fs::copy(&src_dump, &dest)?;
                        database_dumps_scheduled += 1;
                        Some(dest)
                    } else {
                        None
                    }
                } else {
                    warnings.push(format!(
                        "Database `{}` ({}) started fresh; in-place data restore is not supported for Redis.",
                        d.name,
                        d.engine.as_str()
                    ));
                    None
                }
            } else {
                None
            };
            pending_db_restores.push((db_id, persisted_dump));
        }
    }

    // 5. Apps + env vars + volumes + domains
    let mut apps_created = 0usize;
    let mut apps_skipped = 0usize;
    let mut apps_deployments_queued = 0usize;
    let mut env_vars_restored = 0usize;
    let mut domains_restored = 0usize;
    let mut volumes_restored = 0usize;

    for a in &manifest.apps {
        let Some(project_id) = project_id_by_slug.get(&a.project_slug).cloned() else {
            warnings.push(format!(
                "Skipped app `{}`: project `{}` was not found",
                a.name, a.project_slug
            ));
            apps_skipped += 1;
            continue;
        };
        let clash: i64 = sqlx::query_scalar(
            "SELECT (SELECT COUNT(*) FROM apps WHERE project_id = ?1 AND (slug = ?2 OR name = ?3))
                  + (SELECT COUNT(*) FROM databases WHERE project_id = ?1 AND slug = ?2)",
        )
        .bind(&project_id)
        .bind(&a.slug)
        .bind(&a.name)
        .fetch_one(&state.db)
        .await?;
        if clash > 0 {
            apps_skipped += 1;
            warnings.push(format!(
                "Skipped existing app `{}/{}` (use Replace mode to overwrite)",
                a.project_slug, a.name
            ));
            continue;
        }

        let app_id = new_id();
        let fallback_secret = random_token(24);
        let webhook_enc = state.vault.encrypt(if a.webhook_secret.is_empty() {
            &fallback_secret
        } else {
            &a.webhook_secret
        })?;
        sqlx::query(
            "INSERT INTO apps (id, org_id, project_id, server_id, name, slug, source_kind, image,
                               git_url, git_branch, dockerfile_path, port, memory_mb, cpu_millis,
                               health_path, status, webhook_secret, auto_deploy, command,
                               entrypoint, extra_networks, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'idle', ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&app_id)
        .bind(org_id)
        .bind(&project_id)
        .bind(&server_id)
        .bind(&a.name)
        .bind(&a.slug)
        .bind(a.source_kind.as_str())
        .bind(&a.image)
        .bind(&a.git_url)
        .bind(&a.git_branch)
        .bind(&a.dockerfile_path)
        .bind(i64::from(a.port))
        .bind(a.memory_mb.map(i64::from))
        .bind(a.cpu_millis.map(i64::from))
        .bind(&a.health_path)
        .bind(&webhook_enc)
        .bind(if a.auto_deploy { 1i64 } else { 0i64 })
        .bind(&a.command)
        .bind(&a.entrypoint)
        .bind(&a.extra_networks)
        .bind(&at)
        .bind(&at)
        .execute(&state.db)
        .await?;
        apps_created += 1;

        for v in &a.volumes {
            let vid = new_id();
            sqlx::query(
                "INSERT INTO app_volumes (id, app_id, name, mount_path, kind, read_only, owned, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&vid)
            .bind(&app_id)
            .bind(format!("rig-app-{vid}"))
            .bind(&v.mount_path)
            .bind(if v.kind == "bind" { "volume" } else { v.kind.as_str() })
            .bind(if v.read_only { 1i64 } else { 0i64 })
            .bind(if v.owned { 1i64 } else { 0i64 })
            .bind(&at)
            .execute(&state.db)
            .await?;
            volumes_restored += 1;
        }

        for ev in &a.env {
            let enc = state.vault.encrypt(&ev.value)?;
            sqlx::query(
                "INSERT INTO env_vars (id, org_id, app_id, key, value_enc, is_secret, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(new_id())
            .bind(org_id)
            .bind(&app_id)
            .bind(&ev.key)
            .bind(&enc)
            .bind(if ev.is_secret { 1i64 } else { 0i64 })
            .bind(&at)
            .execute(&state.db)
            .await?;
            env_vars_restored += 1;
        }

        for dom in &a.domains {
            let taken: Option<i64> = sqlx::query_scalar(
                "SELECT 1 FROM domains WHERE host = ? COLLATE NOCASE AND ifnull(port, 0) = ifnull(?, 0)",
            )
            .bind(&dom.host)
            .bind(dom.port.map(i64::from))
            .fetch_optional(&state.db)
            .await?;
            if taken.is_some() {
                warnings.push(format!(
                    "Skipped domain `{}` for `{}` because it is already bound on this server",
                    dom.host, a.name
                ));
                continue;
            }
            sqlx::query(
                "INSERT INTO domains (id, org_id, app_id, host, tls_mode, access, port, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(new_id())
            .bind(org_id)
            .bind(&app_id)
            .bind(&dom.host)
            .bind(dom.tls_mode.as_str())
            .bind(dom.access.as_str())
            .bind(dom.port.map(i64::from))
            .bind(&at)
            .execute(&state.db)
            .await?;
            domains_restored += 1;
        }

        if opts.deploy_running_apps && a.was_running {
            match a.source_kind {
                SourceKind::Image | SourceKind::Git => {
                    if deploy::enqueue(
                        state,
                        deploy::NewDeployment {
                            org_id,
                            app_id: &app_id,
                            trigger: "api",
                            created_by: Some(actor_user_id),
                            image_ref: None,
                            image_id: None,
                            source_dir: None,
                            commit_msg: Some("Restored from server snapshot"),
                        },
                    )
                    .await
                    .is_ok()
                    {
                        apps_deployments_queued += 1;
                    }
                }
                SourceKind::Local => {
                    warnings.push(format!(
                        "App `{}` is a local-source app and cannot auto-deploy from snapshot; push its source with `rig up` or Local Git.",
                        a.name
                    ));
                }
            }
        }
    }

    // 6. Alert channels
    let mut alerts_created = 0usize;
    if opts.restore_alerts {
        for ch in &manifest.alert_channels {
            let exists: Option<i64> = sqlx::query_scalar(
                "SELECT 1 FROM alert_channels WHERE org_id = ? AND name = ? COLLATE NOCASE",
            )
            .bind(org_id)
            .bind(&ch.name)
            .fetch_optional(&state.db)
            .await?;
            if exists.is_some() {
                continue;
            }
            sqlx::query(
                "INSERT INTO alert_channels (id, org_id, name, url_enc, url_host, signing_secret, events, enabled, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(new_id())
            .bind(org_id)
            .bind(&ch.name)
            .bind(state.vault.encrypt(&ch.url)?)
            .bind(&ch.url_host)
            .bind(state.vault.encrypt(&ch.signing_secret)?)
            .bind(&ch.events_raw)
            .bind(if ch.enabled { 1i64 } else { 0i64 })
            .bind(&at)
            .execute(&state.db)
            .await?;
            alerts_created += 1;
        }
    }

    // 7. Optional GitHub App
    let mut github_restored = false;
    if opts.restore_github
        && let Some(gh) = &manifest.github_app
    {
        sqlx::query(
            "INSERT OR REPLACE INTO github_apps
             (org_id, app_id, slug, name, html_url, owner, client_id, client_secret, webhook_secret, private_key, created_by, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(org_id)
        .bind(gh.app_id)
        .bind(&gh.slug)
        .bind(&gh.name)
        .bind(&gh.html_url)
        .bind(&gh.owner)
        .bind(&gh.client_id)
        .bind(state.vault.encrypt(&gh.client_secret)?)
        .bind(state.vault.encrypt(&gh.webhook_secret)?)
        .bind(state.vault.encrypt(&gh.private_key)?)
        .bind(actor_user_id)
        .bind(&at)
        .execute(&state.db)
        .await?;
        github_restored = true;
    }

    // Spawn async database provisioning + dump restoration tasks
    for (db_id, dump_path) in pending_db_restores {
        let st = state.clone();
        tokio::spawn(async move {
            databases::provision(st.clone(), db_id.clone()).await;
            if let Some(path) = dump_path {
                // Give newly initialized DB engines a brief settle window before streaming SQL/dump.
                tokio::time::sleep(Duration::from_secs(2)).await;
                let status: Option<String> =
                    sqlx::query_scalar("SELECT status FROM databases WHERE id = ?")
                        .bind(&db_id)
                        .fetch_optional(&st.db)
                        .await
                        .ok()
                        .flatten();
                if status.as_deref() == Some("running") {
                    if let Err(e) = backups::restore_plaintext_from_file(&st, &db_id, &path).await {
                        tracing::warn!(
                            database = %db_id,
                            error = %format!("{e:#}"),
                            "snapshot database dump restore failed"
                        );
                    } else {
                        tracing::info!(database = %db_id, "restored database dump from server snapshot");
                    }
                }
                let _ = tokio::fs::remove_file(&path).await;
            }
        });
    }

    if let Err(e) = proxy::sync(state).await {
        tracing::warn!(error = %format!("{e:#}"), "proxy sync after snapshot restore");
    }

    Ok(SnapshotRestoreReport {
        mode: opts.mode,
        branding_restored,
        media_restored,
        projects_created,
        projects_skipped,
        apps_created,
        apps_skipped,
        apps_deployments_queued,
        env_vars_restored,
        domains_restored,
        volumes_restored,
        databases_created,
        databases_skipped,
        databases_provisioning,
        database_dumps_scheduled,
        alerts_created,
        github_restored,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    struct DummyRuntime;

    #[async_trait::async_trait]
    impl crate::runtime::Runtime for DummyRuntime {
        async fn info(&self) -> Result<crate::runtime::EngineInfo> {
            Ok(crate::runtime::EngineInfo {
                name: "docker".into(),
                version: "27.0".into(),
            })
        }
        async fn pull(&self, _: &str, _: &crate::runtime::LogSink) -> Result<()> {
            Ok(())
        }
        async fn build(&self, _: crate::runtime::BuildSpec, _: &crate::runtime::LogSink) -> Result<()> {
            Ok(())
        }
        async fn build_plan(&self, _: crate::runtime::PlanSpec, _: &crate::runtime::LogSink) -> Result<()> {
            Ok(())
        }
        async fn run(&self, _: &crate::runtime::ContainerSpec) -> Result<crate::runtime::Started> {
            bail!("unused")
        }
        async fn state(&self, _: &str) -> Result<crate::runtime::ContainerState> {
            Ok(crate::runtime::ContainerState::Missing)
        }
        async fn health(&self, _: &str) -> Result<Option<crate::runtime::Health>> {
            Ok(None)
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
        fn logs_with(&self, _: &str, _: crate::runtime::LogOptions) -> crate::runtime::LogStream {
            Box::pin(futures_util::stream::empty())
        }
        async fn list_by_label(&self, _: &str, _: &str) -> Result<Vec<String>> {
            Ok(Vec::new())
        }
        async fn remove_volume(&self, _: &str) -> Result<()> {
            Ok(())
        }
        async fn remove_network(&self, _: &str) -> Result<()> {
            Ok(())
        }
        async fn shell(&self, _: &str, _: u16, _: u16) -> Result<crate::runtime::Shell> {
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

    struct DummyProxy;
    #[async_trait::async_trait]
    impl crate::proxy::Proxy for DummyProxy {
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

    async fn test_server(dir: &Path, org_id: &str, org_name: &str) -> AppState {
        let db = crate::state::open_db(&format!("sqlite://{}", dir.join("rig.db").display()))
            .await
            .unwrap();
        let vault = crate::crypto::Vault::open(dir).unwrap();
        sqlx::query("INSERT INTO organizations (id, name, slug, accent_color, env_label, env_color) VALUES (?, ?, 'acme', 'indigo', 'Production', 'amber')")
            .bind(org_id)
            .bind(org_name)
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO users (id, email, name, password_hash) VALUES ('u1', 'owner@example.com', 'Owner', 'hash')")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO memberships (org_id, user_id, role) VALUES (?, 'u1', 'owner')")
            .bind(org_id)
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO servers (id, org_id, name, kind) VALUES ('srv1', ?, 'local', 'local')")
            .bind(org_id)
            .execute(&db)
            .await
            .unwrap();
        AppState {
            db,
            vault,
            config: Arc::new(crate::state::Config {
                data_dir: dir.to_path_buf(),
                secure_cookies: false,
                github_api: "http://127.0.0.1".into(),
                github_web: "http://127.0.0.1".into(),
                poll_interval: 0,
                allow_private_alerts: false,
                allow_private_git: false,
                enable_api_docs: false,
                mode: rig_api::ServerMode::Full,
            }),
            runtime: Arc::new(DummyRuntime),
            proxy: Arc::new(DummyProxy),
            deploys: Arc::new(crate::deploy::Deployer::new().unwrap()),
            access: crate::access::DashboardAccess::default(),
            caches: Arc::new(crate::state::RuntimeCaches::default()),
            events: Arc::new(crate::events::EventBus::new()),
        }
    }

    #[tokio::test]
    async fn cross_server_encrypted_snapshot_reencrypts_secrets_for_new_vault() {
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let server_a = test_server(dir_a.path(), "org-a", "Source Cloud").await;
        let server_b = test_server(dir_b.path(), "org-b", "Fresh Target").await;

        // Populate server A with a project, app, encrypted env secret, volume, domain, and database
        sqlx::query("INSERT INTO projects (id, org_id, name, slug, description) VALUES ('p1', 'org-a', 'Core', 'core', 'Main workloads')")
            .execute(&server_a.db)
            .await
            .unwrap();
        let wh_enc = server_a.vault.encrypt("wh-secret-123").unwrap();
        sqlx::query(
            "INSERT INTO apps (id, org_id, project_id, server_id, name, slug, source_kind, image, port, memory_mb, cpu_millis, status, webhook_secret)
             VALUES ('a1', 'org-a', 'p1', 'srv1', 'API', 'api', 'image', 'nginx:alpine', 8080, 256, 500, 'stopped', ?)",
        )
        .bind(&wh_enc)
        .execute(&server_a.db)
        .await
        .unwrap();
        let secret_enc = server_a.vault.encrypt("super-secret-token").unwrap();
        sqlx::query(
            "INSERT INTO env_vars (id, org_id, app_id, key, value_enc, is_secret)
             VALUES ('e1', 'org-a', 'a1', 'API_SECRET', ?, 1)",
        )
        .bind(&secret_enc)
        .execute(&server_a.db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO app_volumes (id, app_id, name, mount_path, created_at)
             VALUES ('v1', 'a1', 'rig-app-v1', '/data', '2026-11-24T00:00:00Z')",
        )
        .execute(&server_a.db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO domains (id, org_id, app_id, host, tls_mode, access)
             VALUES ('d1', 'org-a', 'a1', 'api.example.com', 'auto', 'public')",
        )
        .execute(&server_a.db)
        .await
        .unwrap();
        let db_pass_enc = server_a.vault.encrypt("pg-pass-999").unwrap();
        sqlx::query(
            "INSERT INTO databases (id, org_id, project_id, name, slug, engine, version, volume, db_name, username, password_enc, status)
             VALUES ('db1', 'org-a', 'p1', 'MainDB', 'maindb', 'postgres', '17', 'rig-db-1', 'maindb', 'maindb', ?, 'stopped')",
        )
        .bind(&db_pass_enc)
        .execute(&server_a.db)
        .await
        .unwrap();

        // Export encrypted snapshot from Server A
        let exported = export_snapshot(
            &server_a,
            "org-a",
            &SnapshotExportRequest {
                include_database_dumps: false,
                include_alerts: true,
                include_github: false,
                passphrase: Some("my-strong-passphrase".into()),
            },
        )
        .await
        .unwrap();
        assert!(exported.encrypted);
        assert!(exported.size_bytes > 0);

        // Wrong passphrase must fail inspection
        assert!(
            inspect_snapshot(&server_b, "org-b", &exported.archive_path, Some("wrong"))
                .await
                .is_err()
        );

        // Correct passphrase inspects cleanly on Server B
        let preview = inspect_snapshot(
            &server_b,
            "org-b",
            &exported.archive_path,
            Some("my-strong-passphrase"),
        )
        .await
        .unwrap();
        assert!(preview.encrypted);
        assert_eq!(preview.source_org_name, "Source Cloud");
        assert_eq!(preview.projects.len(), 1);
        assert_eq!(preview.apps.len(), 1);
        assert_eq!(preview.databases.len(), 1);

        // Restore onto Server B (which has a completely different master.key!)
        let report = restore_snapshot(
            &server_b,
            "org-b",
            "u1",
            &exported.archive_path,
            Some("my-strong-passphrase"),
            &SnapshotRestoreOptions {
                mode: SnapshotRestoreMode::Merge,
                restore_branding: true,
                restore_alerts: true,
                restore_github: false,
                start_databases: false,
                deploy_running_apps: false,
            },
        )
        .await
        .unwrap();
        assert!(report.branding_restored);
        assert_eq!(report.projects_created, 1);
        assert_eq!(report.apps_created, 1);
        assert_eq!(report.env_vars_restored, 1);
        assert_eq!(report.domains_restored, 1);
        assert_eq!(report.volumes_restored, 1);
        assert_eq!(report.databases_created, 1);

        // Verify Server B's Vault can decrypt the re-encrypted secrets!
        let restored_env_enc: String =
            sqlx::query_scalar("SELECT value_enc FROM env_vars WHERE org_id = 'org-b' AND key = 'API_SECRET'")
                .fetch_one(&server_b.db)
                .await
                .unwrap();
        assert_eq!(
            server_b.vault.decrypt(&restored_env_enc).unwrap(),
            "super-secret-token"
        );
        let restored_db_enc: String =
            sqlx::query_scalar("SELECT password_enc FROM databases WHERE org_id = 'org-b' AND slug = 'maindb'")
                .fetch_one(&server_b.db)
                .await
                .unwrap();
        assert_eq!(
            server_b.vault.decrypt(&restored_db_enc).unwrap(),
            "pg-pass-999"
        );
    }
}
