//! Local Git checkout inspector and source-bundle deployer (§9.1).
//!
//! Lets the user pick a local Git repository folder, see its branch, HEAD
//! commit, remote URL, uncommitted changes, and unpushed commits, then deploy
//! directly to a Rig app via Rig's content-addressed local source upload API.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result, bail};
use ignore::WalkBuilder;
use rig_api::{ContextFileEntry, Deployment, LocalSourceDeployRequest};
use rig_client::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalGitStatus {
    pub path: String,
    pub is_git_repo: bool,
    pub branch: Option<String>,
    pub head_sha: Option<String>,
    pub head_subject: Option<String>,
    pub remote_url: Option<String>,
    pub dirty_files: usize,
    pub unpushed_commits: usize,
    pub has_dockerfile: bool,
    pub stacks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredRepo {
    pub name: String,
    pub path: String,
    pub branch: Option<String>,
    pub head_sha: Option<String>,
    pub head_subject: Option<String>,
    pub remote_url: Option<String>,
    pub dirty_files: usize,
    pub unpushed_commits: usize,
    pub has_dockerfile: bool,
    pub stacks: Vec<String>,
    pub modified_epoch: u64,
}

fn detect_stacks(dir: &Path) -> Vec<String> {
    let mut stacks = Vec::new();
    if dir.join("Dockerfile").is_file() || dir.join("Containerfile").is_file() {
        stacks.push("Docker".into());
    }
    if dir.join("docker-compose.yml").is_file()
        || dir.join("docker-compose.yaml").is_file()
        || dir.join("compose.yml").is_file()
        || dir.join("compose.yaml").is_file()
    {
        stacks.push("Compose".into());
    }
    if dir.join("Cargo.toml").is_file() {
        stacks.push("Rust".into());
    }
    if dir.join("package.json").is_file() {
        stacks.push("Node".into());
    }
    if dir.join("go.mod").is_file() {
        stacks.push("Go".into());
    }
    if dir.join("pyproject.toml").is_file() || dir.join("requirements.txt").is_file() {
        stacks.push("Python".into());
    }
    stacks
}

/// Scans standard developer roots (or user-provided folders) to discover local
/// Git repositories / deployable project folders quickly without descending into
/// build artifacts or dependencies.
pub fn discover_repos(extra_roots: Option<Vec<String>>) -> Vec<DiscoveredRepo> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(extras) = extra_roots {
        for r in extras {
            let p = PathBuf::from(r.trim());
            if p.is_dir() {
                roots.push(p);
            }
        }
    }
    if roots.is_empty()
        && let Some(home) = dirs::home_dir()
    {
        for sub in [
            "code",
            "Code",
            "projects",
            "Projects",
            "src",
            "dev",
            "Dev",
            "workspace",
            "repos",
            "work",
        ] {
            let p = home.join(sub);
            if p.is_dir() {
                roots.push(p);
            }
        }
    }

    let mut seen = HashSet::new();
    let mut found = Vec::new();
    for root in roots {
        scan_dir_for_repos(&root, 0, 3, &mut seen, &mut found);
        if found.len() >= 80 {
            break;
        }
    }
    found.sort_by(|a, b| b.modified_epoch.cmp(&a.modified_epoch).then_with(|| a.name.cmp(&b.name)));
    found
}

fn scan_dir_for_repos(
    dir: &Path,
    depth: usize,
    max_depth: usize,
    seen: &mut HashSet<PathBuf>,
    out: &mut Vec<DiscoveredRepo>,
) {
    if depth > max_depth || out.len() >= 80 {
        return;
    }
    let Ok(canon) = dir.canonicalize() else {
        return;
    };
    if !seen.insert(canon.clone()) {
        return;
    }

    let has_git = dir.join(".git").exists();
    let stacks = detect_stacks(dir);
    let is_project = has_git || !stacks.is_empty();

    if depth > 0 && is_project {
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project")
            .to_owned();
        let modified_epoch = std::fs::metadata(dir.join(".git").join("HEAD"))
            .or_else(|_| std::fs::metadata(dir))
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let (branch, head_sha, head_subject, remote_url, dirty_files, unpushed_commits) = if has_git
        {
            (
                run_git(dir, &["rev-parse", "--abbrev-ref", "HEAD"]),
                run_git(dir, &["rev-parse", "--short=8", "HEAD"]),
                run_git(dir, &["log", "-1", "--pretty=%s"]),
                run_git(dir, &["remote", "get-url", "origin"]),
                run_git(dir, &["status", "--porcelain"])
                    .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
                    .unwrap_or(0),
                run_git(dir, &["rev-list", "--count", "@{u}..HEAD"])
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0),
            )
        } else {
            (None, None, None, None, 0, 0)
        };

        out.push(DiscoveredRepo {
            name,
            path: dir.display().to_string(),
            branch,
            head_sha,
            head_subject,
            remote_url,
            dirty_files,
            unpushed_commits,
            has_dockerfile: dir.join("Dockerfile").is_file() || dir.join("Containerfile").is_file(),
            stacks,
            modified_epoch,
        });
        // Do not descend into subdirs of a git repo.
        if has_git {
            return;
        }
    }

    if depth == max_depth {
        return;
    }

    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.')
            || matches!(
                name.as_ref(),
                "node_modules" | "target" | "dist" | "build" | "vendor" | "__pycache__" | "venv" | ".venv"
            )
        {
            continue;
        }
        scan_dir_for_repos(&path, depth + 1, max_depth, seen, out);
    }
}

fn run_git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

pub fn inspect_repo(path: &str) -> Result<LocalGitStatus> {
    let dir = Path::new(path);
    if !dir.is_dir() {
        bail!("directory does not exist: {path}");
    }
    let has_dockerfile = dir.join("Dockerfile").is_file() || dir.join("Containerfile").is_file();
    let stacks = detect_stacks(dir);
    let is_git = run_git(dir, &["rev-parse", "--is-inside-work-tree"])
        .is_some_and(|s| s == "true");
    if !is_git {
        return Ok(LocalGitStatus {
            path: dir.display().to_string(),
            is_git_repo: false,
            branch: None,
            head_sha: None,
            head_subject: None,
            remote_url: None,
            dirty_files: 0,
            unpushed_commits: 0,
            has_dockerfile,
            stacks,
        });
    }

    let branch = run_git(dir, &["rev-parse", "--abbrev-ref", "HEAD"]);
    let head_sha = run_git(dir, &["rev-parse", "--short=8", "HEAD"]);
    let head_subject = run_git(dir, &["log", "-1", "--pretty=%s"]);
    let remote_url = run_git(dir, &["remote", "get-url", "origin"]);
    let dirty_files = run_git(dir, &["status", "--porcelain"])
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0);
    let unpushed_commits = run_git(dir, &["rev-list", "--count", "@{u}..HEAD"])
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);

    Ok(LocalGitStatus {
        path: dir.display().to_string(),
        is_git_repo: true,
        branch,
        head_sha,
        head_subject,
        remote_url,
        dirty_files,
        unpushed_commits,
        has_dockerfile,
        stacks,
    })
}

/// Uploads the local checkout (honoring `.gitignore`, skipping `.git`) using
/// Rig's deduplicated content-addressed blob API and triggers a deployment.
pub async fn deploy_local_checkout(
    client: &Client,
    app_id: &str,
    repo_path: &str,
    dockerfile_path: Option<String>,
) -> Result<Deployment> {
    let root = Path::new(repo_path).to_path_buf();
    if !root.is_dir() {
        bail!("local path is not a directory: {repo_path}");
    }

    // Walk and hash files off the async runtime.
    let (entries, blobs_by_hash) = tokio::task::spawn_blocking(move || {
        collect_source_entries(&root)
    })
    .await
    .context("scanning local checkout")??;

    if entries.is_empty() {
        bail!("no deployable files found in {repo_path}");
    }

    let unique_hashes: Vec<String> = blobs_by_hash.keys().cloned().collect();
    let check = client
        .check_blobs(&unique_hashes)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // Upload only missing blobs in a single zstd-compressed tar batch.
    if !check.missing.is_empty() {
        let missing = check.missing.clone();
        let batch_bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
            let mut tar_buf = Vec::new();
            {
                let mut builder = tar::Builder::new(&mut tar_buf);
                for hash in &missing {
                    if let Some(raw) = blobs_by_hash.get(hash) {
                        let mut hdr = tar::Header::new_gnu();
                        hdr.set_size(raw.len() as u64);
                        hdr.set_mode(0o644);
                        hdr.set_cksum();
                        builder.append_data(&mut hdr, hash, raw.as_slice())?;
                    }
                }
                builder.finish()?;
            }
            let compressed = zstd::encode_all(tar_buf.as_slice(), 3)?;
            Ok(compressed)
        })
        .await
        .context("compressing source batch")??;

        client
            .upload_blob_batch_zstd(batch_bytes)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
    }

    let label = Path::new(repo_path)
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_owned);
    let deployment = client
        .deploy_local_source(
            app_id,
            &LocalSourceDeployRequest {
                files: entries,
                dockerfile: dockerfile_path,
                label,
            },
        )
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    Ok(deployment)
}

fn collect_source_entries(
    root: &Path,
) -> Result<(Vec<ContextFileEntry>, HashMap<String, Vec<u8>>)> {
    let mut entries = Vec::new();
    let mut blobs = HashMap::new();
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(true)
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            name != ".git" && name != "node_modules" && name != "target"
        })
        .build();

    for item in walker {
        let entry = item?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        if rel.is_empty() {
            continue;
        }
        let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let sha = hex::encode(Sha256::digest(&data));
        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(path)
                .map(|m| m.permissions().mode() & 0o777)
                .unwrap_or(0o644)
        };
        #[cfg(not(unix))]
        let mode = 0o644u32;

        let size = data.len() as u64;
        blobs.entry(sha.clone()).or_insert(data);
        entries.push(ContextFileEntry {
            path: rel,
            sha256: sha,
            mode,
            size,
        });
    }
    Ok((entries, blobs))
}
