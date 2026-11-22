//! Builds for repositories without a Dockerfile, using [Railpack].
//!
//! Railpack reads the source and writes a build plan. The plan is then built
//! by BuildKit through Railpack's frontend image, so nothing stays running
//! between builds. The CLI is fetched on first use from a pinned release and
//! checked against a pinned SHA-256 before it is ever run.
//!
//! [Railpack]: https://railpack.com

use std::{
    io::Read,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::runtime::LogSink;

pub const VERSION: &str = "0.43.0";

/// BuildKit frontend that turns a Railpack plan into an image. Pinned by
/// digest so a moved tag cannot change what runs.
pub const FRONTEND: &str = "ghcr.io/railwayapp/railpack-frontend:v0.43.0@sha256:355ef61fae6f2551103cd8b10c710cb07167dd353d561f15c69b8d7ac1c676cc";

/// Release asset name and SHA-256 for each supported architecture.
fn asset() -> Result<(&'static str, &'static str)> {
    Ok(match std::env::consts::ARCH {
        "x86_64" => (
            "railpack-v0.43.0-x86_64-unknown-linux-musl.tar.gz",
            "248a8f60f36854aabed6124113631078a802de3461660855fc4de9c26065c1fb",
        ),
        "aarch64" => (
            "railpack-v0.43.0-arm64-unknown-linux-musl.tar.gz",
            "732ece14eb4f4676a43ebba19431f954edf7b130acb76159ef0f2ff86ac42ac9",
        ),
        other => bail!("Railpack has no build for {other}. Add a Dockerfile to the repository"),
    })
}

const DOWNLOAD_LIMIT: u64 = 64 * 1024 * 1024;
const PREPARE_TIMEOUT: Duration = Duration::from_secs(120);

/// Only one deploy fetches the binary at a time.
static FETCH: Mutex<()> = Mutex::const_new(());

/// Path to a usable `railpack`: `<data_dir>/bin/railpack`, then `PATH`,
/// otherwise the pinned release is downloaded into `<data_dir>/bin`.
pub async fn binary(data_dir: &Path, log: &LogSink) -> Result<PathBuf> {
    let local = data_dir.join("bin").join("railpack");
    if is_file(&local).await {
        return Ok(local);
    }
    if let Some(found) = on_path().await {
        return Ok(found);
    }
    let _guard = FETCH.lock().await;
    if is_file(&local).await {
        return Ok(local);
    }
    log.line("build", format!("Fetching Railpack v{VERSION}"))
        .await;
    fetch(&local).await?;
    Ok(local)
}

async fn is_file(p: &Path) -> bool {
    tokio::fs::metadata(p).await.is_ok_and(|m| m.is_file())
}

async fn on_path() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join("railpack");
        if is_file(&candidate).await {
            return Some(candidate);
        }
    }
    None
}

async fn fetch(dest: &Path) -> Result<()> {
    let (name, sha) = asset()?;
    let url = format!("https://github.com/railwayapp/railpack/releases/download/v{VERSION}/{name}");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()?;
    let resp = client
        .get(&url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .context("downloading Railpack")?;
    if resp.content_length().is_some_and(|n| n > DOWNLOAD_LIMIT) {
        bail!("the Railpack download is larger than expected");
    }
    let bytes = resp.bytes().await.context("downloading Railpack")?;
    if bytes.len() as u64 > DOWNLOAD_LIMIT {
        bail!("the Railpack download is larger than expected");
    }
    let got = hex::encode(Sha256::digest(&bytes));
    if got != sha {
        bail!("the Railpack download failed its checksum (got {got})");
    }
    let dest = dest.to_path_buf();
    tokio::task::spawn_blocking(move || unpack(&bytes, &dest))
        .await
        .context("unpacking Railpack")?
}

/// Write the `railpack` entry of the archive to `dest`, atomically.
fn unpack(archive: &[u8], dest: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let dir = dest.parent().context("no parent directory")?;
    std::fs::create_dir_all(dir)?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        if path.file_name().is_some_and(|n| n == "railpack")
            && entry.header().entry_type().is_file()
        {
            let mut data = Vec::new();
            entry
                .by_ref()
                .take(DOWNLOAD_LIMIT * 4)
                .read_to_end(&mut data)?;
            let tmp = tempfile::NamedTempFile::new_in(dir)?;
            std::fs::write(tmp.path(), &data)?;
            std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o755))?;
            tmp.persist(dest).context("installing Railpack")?;
            return Ok(());
        }
    }
    bail!("the Railpack archive has no railpack binary")
}

/// Run `railpack prepare` on `source`, writing `railpack-plan.json` into
/// `plan_dir`. Railpack only reads the source here; it runs nothing from it.
pub async fn prepare(bin: &Path, source: &Path, plan_dir: &Path, log: &LogSink) -> Result<()> {
    tokio::fs::create_dir_all(plan_dir).await?;
    let mut cmd = tokio::process::Command::new(bin);
    cmd.arg("prepare")
        .arg("--plan-out")
        .arg(plan_dir.join("railpack-plan.json"))
        .arg("--info-out")
        .arg(plan_dir.join("railpack-info.json"))
        .arg("--error-missing-start")
        .arg("--")
        .arg(source)
        // Keep the server's own environment (and anything secret in it) out
        // of the plan: only what Railpack needs to run.
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", plan_dir)
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    let out = tokio::time::timeout(PREPARE_TIMEOUT, cmd.output())
        .await
        .context("Railpack took too long to read the repository")?
        .context("running Railpack")?;
    for line in String::from_utf8_lossy(&out.stdout)
        .lines()
        .chain(String::from_utf8_lossy(&out.stderr).lines())
    {
        log.line("build", line).await;
    }
    if !out.status.success() {
        bail!(
            "Railpack could not work out how to build this repository. Add a Dockerfile, or a railpack.json (https://railpack.com/config/file)"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        {
            let mut b = tar::Builder::new(&mut gz);
            for (name, data) in entries {
                let mut h = tar::Header::new_gnu();
                h.set_size(data.len() as u64);
                h.set_mode(0o644);
                h.set_cksum();
                b.append_data(&mut h, name, *data).unwrap();
            }
            b.finish().unwrap();
        }
        gz.finish().unwrap()
    }

    #[test]
    fn unpacks_only_the_binary() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("bin").join("railpack");
        let a = archive(&[("README.md", b"hi"), ("railpack", b"#!bin")]);
        unpack(&a, &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"#!bin");
        assert!(!dir.path().join("bin").join("README.md").exists());
    }

    #[test]
    fn refuses_an_archive_without_the_binary() {
        let dir = tempfile::tempdir().unwrap();
        let a = archive(&[("README.md", b"hi")]);
        assert!(unpack(&a, &dir.path().join("railpack")).is_err());
    }
}
