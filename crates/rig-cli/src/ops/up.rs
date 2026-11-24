//! Local folder and direct binary deploy engine (`rig up` & MCP `deploy_local`).
//!
//! Supports three modes:
//! 1. **Direct Binary Mode:** pass a pre-compiled Linux ELF binary; Rig hashes it,
//!    deduplicates against the server blob cache, streams it `zstd`-compressed, and
//!    wraps it in a minimal `scratch` (static) or `distroless` (dynamic) OCI image.
//! 2. **Source Mode (local build — default):** builds the OCI image locally using
//!    `podman` or `docker`, extracts layer blobs, checks the server's blob cache,
//!    and streams ONLY changed `zstd`-compressed layers.
//! 3. **Source Mode (remote build — `--remote-build` or fallback):** walks the
//!    working directory with the `ignore` crate (respecting `.gitignore` and
//!    `.dockerignore`), hashes files, packs ONLY changed files into a single
//!    `zstd`-compressed delta tarball, and builds remotely via Dockerfile,
//!    Containerfile, or Railpack.

use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    process::Stdio,
};

use anyhow::Context as _;
use ignore::WalkBuilder;
use rig_api::{
    App, ArchiveBlobEntry, BinaryDeployRequest, ContextFileEntry, CreateAppRequest, Deployment,
    LocalImageDeployRequest, LocalSourceDeployRequest, SourceKind,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::process::Command;

use crate::{
    api::Client,
    error::{CliError, CliResult},
    link, ops,
};

/// Options for executing `rig up` from either the CLI or MCP.
#[derive(Debug, Clone, Default)]
pub struct UpOptions {
    /// Directory or binary path (defaults to current working directory).
    pub path: Option<PathBuf>,
    /// Explicit pre-compiled Linux binary path (forces Direct Binary Mode).
    pub binary: Option<PathBuf>,
    /// Target app name, `project/app`, or id. Defaults to `rig.toml` or directory name.
    pub app: Option<String>,
    /// Project name, slug, or id (used when creating a new local app on first `rig up`).
    pub project: Option<String>,
    /// Force remote build on the Rig server instead of building locally.
    pub remote_build: bool,
    /// Optional container port override.
    pub port: Option<u16>,
    /// Optional Dockerfile/Containerfile path relative to context root.
    pub dockerfile: Option<String>,
    /// Optional HTTP health check path.
    pub health_path: Option<String>,
    /// Optional base image override for direct binary deploys.
    pub base_image: Option<String>,
    /// Optional default command-line args for direct binary deploys.
    pub binary_args: Vec<String>,
}

/// Summary of what `execute_up` uploaded and triggered.
#[derive(Debug, Clone, Serialize)]
pub struct UpSummary {
    pub mode: String,
    pub app: App,
    pub deployment: Deployment,
    pub total_assets: usize,
    pub uploaded_assets: usize,
    pub cached_assets: usize,
    pub uploaded_bytes_compressed: u64,
}

/// A scanned file from a local project directory.
#[derive(Debug, Clone)]
pub struct ScannedFile {
    pub abs_path: PathBuf,
    pub rel_path: String,
    pub sha256: String,
    pub mode: u32,
    pub size: u64,
}

/// Scans `dir` using the `ignore` crate, respecting `.gitignore` and `.dockerignore`,
/// always skipping `.git/`, and computing SHA-256 content hashes for deduplication.
pub fn scan_context(dir: &Path, explicit_dockerfile: Option<&str>) -> CliResult<Vec<ScannedFile>> {
    if !dir.is_dir() {
        return Err(CliError::message(format!(
            "{} is not a directory",
            dir.display()
        )));
    }
    let root = dir
        .canonicalize()
        .with_context(|| format!("resolving directory {}", dir.display()))?;

    let mut builder = WalkBuilder::new(&root);
    builder
        .hidden(false) // Keep dotfiles like .env.example, .dockerignore, etc.
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .ignore(true)
        .parents(true)
        .follow_links(false)
        .add_custom_ignore_filename(".dockerignore")
        .filter_entry(|entry| entry.file_name() != ".git");

    let mut files = Vec::new();
    let mut seen_rel = HashSet::new();

    for result in builder.build() {
        let entry = match result {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path == root {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(path) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let Ok(rel) = path.strip_prefix(&root) else {
            continue;
        };
        let rel_str = to_posix_rel(rel);
        if rel_str.is_empty() || rel_str == ".git" || rel_str.starts_with(".git/") {
            continue;
        }
        if !seen_rel.insert(rel_str.clone()) {
            continue;
        }
        files.push(hash_local_file(path, rel_str)?);
    }

    // Always ensure Dockerfile, Containerfile, explicit dockerfile, and .dockerignore
    // are included even if matched by an ignore rule (matching Docker/Podman semantics).
    let mut must_keep: Vec<String> = vec![
        "Dockerfile".into(),
        "Containerfile".into(),
        ".dockerignore".into(),
    ];
    if let Some(df) = explicit_dockerfile {
        must_keep.push(df.trim_start_matches("./").to_string());
    }
    for rel_str in must_keep {
        if seen_rel.contains(&rel_str) {
            continue;
        }
        let candidate = root.join(&rel_str);
        if let Ok(meta) = std::fs::symlink_metadata(&candidate)
            && meta.is_file()
        {
            seen_rel.insert(rel_str.clone());
            files.push(hash_local_file(&candidate, rel_str)?);
        }
    }

    files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(files)
}

fn hash_local_file(abs_path: &Path, rel_path: String) -> CliResult<ScannedFile> {
    let mut file =
        File::open(abs_path).with_context(|| format!("reading {}", abs_path.display()))?;
    let meta = file.metadata()?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o777
    };
    #[cfg(not(unix))]
    let mode = 0o644;

    Ok(ScannedFile {
        abs_path: abs_path.to_path_buf(),
        rel_path,
        sha256: hex::encode(hasher.finalize()),
        mode,
        size: meta.len(),
    })
}

fn to_posix_rel(rel: &Path) -> String {
    rel.components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Checks if a local file starts with the Linux ELF magic bytes (`\x7fELF`).
pub fn validate_local_elf(path: &Path) -> CliResult<()> {
    let mut f = File::open(path).with_context(|| format!("opening binary {}", path.display()))?;
    let mut magic = [0u8; 4];
    let n = f.read(&mut magic)?;
    if n < 4 || &magic != b"\x7fELF" {
        return Err(CliError::message(format!(
            "{} is not a Linux ELF binary. Cross-compile for Linux first (e.g. `cargo build --release --target x86_64-unknown-linux-musl` or `GOOS=linux GOARCH=amd64 go build`)",
            path.display()
        )));
    }
    Ok(())
}

/// Detects an available local container CLI (`podman` or `docker`) for local builds.
pub async fn detect_local_engine() -> Option<&'static str> {
    for candidate in ["podman", "docker"] {
        let ok = Command::new(candidate)
            .arg("version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .is_ok_and(|s| s.success());
        if ok {
            return Some(candidate);
        }
    }
    None
}

/// Finds the Dockerfile or Containerfile inside `dir`.
pub fn detect_dockerfile(dir: &Path, explicit: Option<&str>) -> Option<String> {
    if let Some(df) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        return dir.join(df).is_file().then(|| df.to_string());
    }
    for name in ["Dockerfile", "Containerfile"] {
        if dir.join(name).is_file() {
            return Some(name.to_string());
        }
    }
    None
}

/// Resolves or creates the target Rig app for `rig up`.
async fn resolve_or_create_app(
    api: &Client,
    opts: &UpOptions,
    link_cfg: Option<&link::RigLink>,
    default_name: &str,
    on_status: &mut (dyn FnMut(&str) + Send),
) -> CliResult<App> {
    let explicit_app = opts
        .app
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    // 1. Try resolving explicit `--app` or `rig.toml` app reference.
    let target_ref = explicit_app
        .map(str::to_owned)
        .or_else(|| link_cfg.map(|l| l.app_ref()));

    if let Some(ref_str) = &target_ref {
        match ops::apps::resolve_app(api, ref_str).await {
            Ok(app) => return Ok(app),
            Err(CliError::NotFound(_)) => {
                // Fall through to auto-create below.
            }
            Err(e) => return Err(e),
        }
    }

    // 2. Determine app name & project for auto-creation on first `rig up`.
    let raw_name = explicit_app
        .map(|s| s.rsplit_once('/').map_or(s, |(_, n)| n))
        .or_else(|| link_cfg.map(|l| l.app.as_str()))
        .unwrap_or(default_name);
    let app_name = sanitize_app_name(raw_name);

    // Try finding an existing app by the derived default name before creating a new one.
    if target_ref.is_none()
        && let Ok(existing) = ops::apps::resolve_app(api, &app_name).await
    {
        return Ok(existing);
    }

    let project_ref = opts
        .project
        .as_deref()
        .or_else(|| {
            explicit_app.and_then(|s| s.split_once('/').map(|(p, _)| p))
        })
        .or_else(|| link_cfg.and_then(|l| l.project.as_deref()));

    let project = match project_ref {
        Some(p) => ops::projects::resolve_project(api, p).await?,
        None => {
            let projects = api.list_projects().await?;
            match projects.len() {
                0 => {
                    return Err(CliError::message(
                        "no projects exist on this server yet. Create one with: rig projects create <name>",
                    ));
                }
                1 => projects.into_iter().next().unwrap(),
                _ => {
                    let names: Vec<String> = projects.iter().map(|p| p.name.clone()).collect();
                    return Err(CliError::message(format!(
                        "no app linked (`rig.toml`) and multiple projects exist ({}). Pass --project <name> or --app <project/app>",
                        names.join(", ")
                    )));
                }
            }
        }
    };

    let port = opts
        .port
        .or_else(|| link_cfg.and_then(|l| l.port))
        .unwrap_or(8080);
    let dockerfile = opts
        .dockerfile
        .clone()
        .or_else(|| link_cfg.and_then(|l| l.dockerfile.clone()));
    let health_path = opts
        .health_path
        .clone()
        .or_else(|| link_cfg.and_then(|l| l.health_path.clone()));

    on_status(&format!(
        "Creating local app `{}` in project `{}` (port {port})",
        app_name, project.name
    ));
    api.create_app(&CreateAppRequest {
        project_id: project.id,
        name: app_name,
        source_kind: SourceKind::Local,
        image: None,
        git_url: None,
        git_branch: None,
        dockerfile_path: dockerfile,
        port,
        memory_mb: link_cfg.and_then(|l| l.memory_mb),
        cpu_millis: link_cfg.and_then(|l| l.cpu_millis),
        health_path,
        volumes: Vec::new(),
        env: Vec::new(),
    })
    .await
}

fn sanitize_app_name(raw: &str) -> String {
    let cleaned: String = raw
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ' ') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches(['-', '_', ' ']);
    if trimmed.is_empty() {
        "local-app".into()
    } else {
        trimmed.chars().take(64).collect()
    }
}

/// Executes `rig up` (Direct Binary Mode, Local Build OCI Mode, or Remote Delta Build Mode).
pub async fn execute_up(
    api: &Client,
    opts: &UpOptions,
    mut on_status: impl FnMut(&str) + Send,
) -> CliResult<UpSummary> {
    let cwd = std::env::current_dir()?;
    let target_path = opts.path.clone().unwrap_or_else(|| cwd.clone());

    // Discover `rig.toml` relative to the target directory (or binary's parent).
    let link_start = if target_path.is_dir() {
        target_path.as_path()
    } else {
        target_path.parent().unwrap_or(&cwd)
    };
    let link_cfg = link::discover_from(link_start)?.map(|(_, l)| l);

    // Determine if this is Direct Binary Mode:
    // 1. Explicit `--binary <path>`
    // 2. Positional `[PATH]` is a regular file (e.g. `rig up ./target/.../app`)
    // 3. `binary = "..."` in `rig.toml` (when no explicit directory path was passed)
    let binary_path = if let Some(b) = &opts.binary {
        Some(b.clone())
    } else if target_path.is_file() {
        Some(target_path.clone())
    } else if opts.path.is_none()
        && let Some(b) = link_cfg.as_ref().and_then(|l| l.binary.as_deref())
    {
        Some(PathBuf::from(b))
    } else {
        None
    };

    if let Some(bin_path) = binary_path {
        return run_binary_mode(api, &bin_path, opts, link_cfg.as_ref(), &mut on_status).await;
    }

    if !target_path.is_dir() {
        return Err(CliError::message(format!(
            "path `{}` does not exist",
            target_path.display()
        )));
    }

    let default_name = target_path
        .canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "local-app".into());

    let app = resolve_or_create_app(api, opts, link_cfg.as_ref(), &default_name, &mut on_status)
        .await?;

    let remote_build = opts.remote_build
        || link_cfg
            .as_ref()
            .and_then(|l| l.remote_build)
            .unwrap_or(false);
    let dockerfile_override = opts
        .dockerfile
        .as_deref()
        .or_else(|| link_cfg.as_ref().and_then(|l| l.dockerfile.as_deref()));
    let detected_df = detect_dockerfile(&target_path, dockerfile_override);

    if !remote_build
        && let Some(df) = detected_df.as_deref()
        && let Some(engine) = detect_local_engine().await
    {
        return run_local_image_mode(
            api,
            &app,
            engine,
            &target_path,
            df,
            &default_name,
            &mut on_status,
        )
        .await;
    }

    if !remote_build && detected_df.is_some() {
        on_status("No local Docker/Podman CLI detected — falling back to remote delta build");
    }
    run_remote_source_mode(
        api,
        &app,
        &target_path,
        detected_df.as_deref(),
        &default_name,
        &mut on_status,
    )
    .await
}

/// Direct Binary Mode: validate ELF, hash, dedup, zstd-stream, wrap in scratch/distroless.
async fn run_binary_mode(
    api: &Client,
    bin_path: &Path,
    opts: &UpOptions,
    link_cfg: Option<&link::RigLink>,
    on_status: &mut (dyn FnMut(&str) + Send),
) -> CliResult<UpSummary> {
    if !bin_path.is_file() {
        return Err(CliError::message(format!(
            "binary `{}` does not exist or is not a file",
            bin_path.display()
        )));
    }
    validate_local_elf(bin_path)?;

    let filename = bin_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "app".into());
    let app =
        resolve_or_create_app(api, opts, link_cfg, &filename, on_status).await?;

    on_status(&format!("Hashing binary `{filename}`"));
    let scanned = hash_local_file(bin_path, filename.clone())?;
    let check = api.check_blobs(std::slice::from_ref(&scanned.sha256)).await?;

    let (uploaded, cached, compressed_bytes) = if check.missing.is_empty() {
        on_status("Binary already cached on server (0 B uploaded)");
        (0, 1, 0)
    } else {
        on_status(&format!(
            "Compressing and streaming `{filename}` with zstd ({} raw)",
            crate::output::bytes(scanned.size)
        ));
        let raw = std::fs::read(bin_path)?;
        let zst = zstd::stream::encode_all(raw.as_slice(), 3)
            .context("compressing binary with zstd")?;
        let comp_len = zst.len() as u64;
        api.upload_blob_zstd(&scanned.sha256, zst).await?;
        on_status(&format!(
            "Uploaded {} compressed",
            crate::output::bytes(comp_len)
        ));
        (1, 0, comp_len)
    };

    on_status("Wrapping binary into minimal OCI image and starting zero-downtime swap");
    let deployment = api
        .deploy_binary(
            &app.id,
            &BinaryDeployRequest {
                sha256: scanned.sha256,
                filename,
                args: opts.binary_args.clone(),
                base_image: opts.base_image.clone(),
                port: opts.port.or_else(|| link_cfg.and_then(|l| l.port)),
            },
        )
        .await?;

    Ok(UpSummary {
        mode: "binary".into(),
        app,
        deployment,
        total_assets: 1,
        uploaded_assets: uploaded,
        cached_assets: cached,
        uploaded_bytes_compressed: compressed_bytes,
    })
}

/// Source Mode (Local Build): build OCI image locally, split saved tar into
/// content-addressed blobs, upload ONLY changed zstd-compressed layers.
async fn run_local_image_mode(
    api: &Client,
    app: &App,
    engine: &str,
    dir: &Path,
    dockerfile: &str,
    label: &str,
    on_status: &mut (dyn FnMut(&str) + Send),
) -> CliResult<UpSummary> {
    let ts = chrono::Utc::now().timestamp();
    let tag = format!("rig/{}:up-{ts}", app.slug);

    on_status(&format!(
        "Building OCI image locally with `{engine}` ({dockerfile}) -> {tag}"
    ));
    let build_status = Command::new(engine)
        .arg("build")
        .arg("-f")
        .arg(dir.join(dockerfile))
        .arg("-t")
        .arg(&tag)
        .arg(dir)
        .stdin(Stdio::null())
        .status()
        .await
        .with_context(|| format!("running `{engine} build`"))?;
    if !build_status.success() {
        return Err(CliError::deploy_failed(format!(
            "local `{engine} build` failed with {build_status}"
        )));
    }

    let tmp_dir = tempfile::tempdir().context("creating temp dir for image export")?;
    let saved_tar = tmp_dir.path().join("image.tar");

    on_status(&format!("Exporting OCI layers from `{engine}`"));
    let save_status = Command::new(engine)
        .args(["save", "-o"])
        .arg(&saved_tar)
        .arg(&tag)
        .stdin(Stdio::null())
        .status()
        .await
        .with_context(|| format!("running `{engine} save`"))?;
    if !save_status.success() {
        return Err(CliError::deploy_failed(format!(
            "`{engine} save` failed with {save_status}"
        )));
    }

    // Extract entries from the saved tar and compute SHA-256 per entry blob.
    let mut entries: Vec<ArchiveBlobEntry> = Vec::new();
    let mut blobs_by_hash: HashMap<String, PathBuf> = HashMap::new();
    let blobs_dir = tmp_dir.path().join("blobs");
    std::fs::create_dir_all(&blobs_dir)?;

    {
        let mut archive = tar::Archive::new(File::open(&saved_tar)?);
        for item in archive.entries()? {
            let mut item = item?;
            if !item.header().entry_type().is_file() {
                continue;
            }
            let rel = item.path()?.to_string_lossy().trim_start_matches("./").to_string();
            let mode = item.header().mode().unwrap_or(0o644);
            let mut data = Vec::with_capacity(item.size().min(16 * 1024 * 1024) as usize);
            item.read_to_end(&mut data)?;
            let sha = hex::encode(Sha256::digest(&data));
            blobs_by_hash.entry(sha.clone()).or_insert_with(|| {
                let p = blobs_dir.join(&sha);
                let _ = std::fs::write(&p, &data);
                p
            });
            entries.push(ArchiveBlobEntry {
                archive_path: rel,
                sha256: sha,
                mode,
            });
        }
    }

    let unique_hashes: Vec<String> = blobs_by_hash.keys().cloned().collect();
    let check = api.check_blobs(&unique_hashes).await?;
    let total_assets = unique_hashes.len();
    let cached_assets = check.present.len();
    let uploaded_assets = check.missing.len();

    on_status(&format!(
        "Layer diff: {uploaded_assets} changed, {cached_assets} already cached on server"
    ));

    let mut total_compressed = 0u64;
    for (idx, sha) in check.missing.iter().enumerate() {
        if let Some(blob_file) = blobs_by_hash.get(sha) {
            let raw = std::fs::read(blob_file)?;
            let zst = zstd::stream::encode_all(raw.as_slice(), 3)
                .context("compressing image layer with zstd")?;
            let comp_len = zst.len() as u64;
            on_status(&format!(
                "Streaming layer {}/{} ({} zstd)",
                idx + 1,
                uploaded_assets,
                crate::output::bytes(comp_len)
            ));
            api.upload_blob_zstd(sha, zst).await?;
            total_compressed += comp_len;
        }
    }

    on_status("Reassembling image on server and starting zero-downtime swap");
    let deployment = api
        .deploy_local_image(
            &app.id,
            &LocalImageDeployRequest {
                image_ref: tag,
                entries,
                label: Some(label.to_string()),
            },
        )
        .await?;

    Ok(UpSummary {
        mode: "local-image".into(),
        app: app.clone(),
        deployment,
        total_assets,
        uploaded_assets,
        cached_assets,
        uploaded_bytes_compressed: total_compressed,
    })
}

/// Source Mode (Remote Delta Build): scan directory respecting `.gitignore` +
/// `.dockerignore`, upload ONLY changed files as a single zstd delta tarball,
/// and build on the server.
async fn run_remote_source_mode(
    api: &Client,
    app: &App,
    dir: &Path,
    dockerfile: Option<&str>,
    label: &str,
    on_status: &mut (dyn FnMut(&str) + Send),
) -> CliResult<UpSummary> {
    on_status(&format!(
        "Scanning {} (respecting .gitignore & .dockerignore)",
        dir.display()
    ));
    let scanned = scan_context(dir, dockerfile)?;
    if scanned.is_empty() {
        return Err(CliError::message(
            "no deployable files found in this directory",
        ));
    }

    let mut by_hash: HashMap<String, &ScannedFile> = HashMap::new();
    for f in &scanned {
        by_hash.entry(f.sha256.clone()).or_insert(f);
    }
    let unique_hashes: Vec<String> = by_hash.keys().cloned().collect();
    let check = api.check_blobs(&unique_hashes).await?;
    let total_assets = unique_hashes.len();
    let cached_assets = check.present.len();
    let uploaded_assets = check.missing.len();

    let mut compressed_bytes = 0u64;
    if check.missing.is_empty() {
        on_status(&format!(
            "All {total_assets} file(s) already cached on server (0 B delta)"
        ));
    } else {
        on_status(&format!(
            "Packing {uploaded_assets} changed file(s) ({cached_assets} cached) into zstd delta tarball"
        ));
        let mut tar_buf = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_buf);
            for sha in &check.missing {
                if let Some(sf) = by_hash.get(sha) {
                    let mut f = File::open(&sf.abs_path)?;
                    let meta = f.metadata()?;
                    let mut hdr = tar::Header::new_gnu();
                    hdr.set_size(meta.len());
                    hdr.set_mode(0o644);
                    hdr.set_cksum();
                    builder.append_data(&mut hdr, sha, &mut f)?;
                }
            }
            builder.finish()?;
        }
        let zst = zstd::stream::encode_all(tar_buf.as_slice(), 3)
            .context("compressing delta tarball with zstd")?;
        compressed_bytes = zst.len() as u64;
        on_status(&format!(
            "Uploading delta ({} zstd)",
            crate::output::bytes(compressed_bytes)
        ));
        api.upload_blob_batch_zstd(zst).await?;
    }

    let manifest: Vec<ContextFileEntry> = scanned
        .into_iter()
        .map(|f| ContextFileEntry {
            path: f.rel_path,
            sha256: f.sha256,
            mode: f.mode,
            size: f.size,
        })
        .collect();

    on_status("Triggering remote build and zero-downtime swap");
    let deployment = api
        .deploy_local_source(
            &app.id,
            &LocalSourceDeployRequest {
                files: manifest,
                dockerfile: dockerfile.map(str::to_owned),
                label: Some(label.to_string()),
            },
        )
        .await?;

    Ok(UpSummary {
        mode: "remote-source".into(),
        app: app.clone(),
        deployment,
        total_assets,
        uploaded_assets,
        cached_assets,
        uploaded_bytes_compressed: compressed_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_context_respects_gitignore_and_dockerignore_and_skips_git() {
        let dir = tempfile::tempdir().unwrap();
        // Initialize a `.git` folder so `ignore` crate activates `.gitignore` rules.
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/config"), "secret").unwrap();
        std::fs::write(dir.path().join(".gitignore"), "target/\n*.log\n").unwrap();
        std::fs::write(dir.path().join(".dockerignore"), "docs/\n").unwrap();
        std::fs::write(dir.path().join("Dockerfile"), "FROM scratch\n").unwrap();
        std::fs::create_dir_all(dir.path().join("target/debug")).unwrap();
        std::fs::write(dir.path().join("target/debug/bin"), "ignored").unwrap();
        std::fs::write(dir.path().join("app.log"), "ignored").unwrap();
        std::fs::create_dir_all(dir.path().join("docs")).unwrap();
        std::fs::write(dir.path().join("docs/guide.md"), "ignored").unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();

        let files = scan_context(dir.path(), None).unwrap();
        let names: Vec<&str> = files.iter().map(|f| f.rel_path.as_str()).collect();
        assert!(names.contains(&"Dockerfile"));
        assert!(names.contains(&"src/main.rs"));
        assert!(!names.iter().any(|n| n.starts_with(".git/")));
        assert!(!names.iter().any(|n| n.starts_with("target/")));
        assert!(!names.iter().any(|n| n.ends_with(".log")));
        assert!(!names.iter().any(|n| n.starts_with("docs/")));
    }

    #[test]
    fn elf_validation_accepts_elf_and_rejects_text() {
        let dir = tempfile::tempdir().unwrap();
        let elf = dir.path().join("app");
        let mut bytes = vec![0u8; 64];
        bytes[0..4].copy_from_slice(b"\x7fELF");
        std::fs::write(&elf, &bytes).unwrap();
        assert!(validate_local_elf(&elf).is_ok());

        let txt = dir.path().join("script.sh");
        std::fs::write(&txt, "#!/bin/sh\n").unwrap();
        assert!(validate_local_elf(&txt).is_err());
    }
}
