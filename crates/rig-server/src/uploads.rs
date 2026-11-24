//! Content-addressed blob cache, delta context staging, local OCI image
//! reassembly, and zero-Dockerfile Linux binary image wrapping for `rig up`.

use std::{
    collections::HashMap,
    fs::{self, File},
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use rig_api::{ArchiveBlobEntry, ContextFileEntry};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::{
    deploy::{LABEL_APP, LABEL_MANAGED},
    runtime::{BuildSpec, LogSink, Runtime},
};

/// Maximum single decompressed blob size accepted by the server (512 MiB).
pub const MAX_BLOB_BYTES: u64 = 512 * 1024 * 1024;
/// Maximum total decompressed bytes in one batch delta tarball (256 MiB).
pub const MAX_BATCH_BYTES: u64 = 256 * 1024 * 1024;
/// Maximum number of files allowed in a single local context manifest.
pub const MAX_CONTEXT_FILES: usize = 50_000;
/// Default minimal base image for dynamically linked Linux ELF binaries.
pub const DEFAULT_DYNAMIC_BASE_IMAGE: &str = "gcr.io/distroless/cc-debian12:nonroot";

/// Validates that `hash` is a strict 64-character lowercase SHA-256 hex digest.
/// This prevents any directory traversal when mapping hashes to blob filenames.
pub fn validate_sha256(hash: &str) -> Result<()> {
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        bail!("invalid sha256 digest: expected 64 lowercase hex characters");
    }
    Ok(())
}

/// Directory storing content-addressed upload blobs: `<data_dir>/uploads/blobs`.
pub fn blobs_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("uploads").join("blobs")
}

/// Directory storing staged remote build contexts: `<data_dir>/uploads/contexts`.
pub fn contexts_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("uploads").join("contexts")
}

/// Path to one blob inside `<data_dir>/uploads/blobs/<prefix>/<sha256>`.
pub fn blob_path(data_dir: &Path, sha256: &str) -> Result<PathBuf> {
    validate_sha256(sha256)?;
    Ok(blobs_dir(data_dir).join(&sha256[..2]).join(sha256))
}

/// Partition `hashes` into `(present, missing)` against the local blob cache.
pub fn check_blobs(data_dir: &Path, hashes: &[String]) -> Result<(Vec<String>, Vec<String>)> {
    let mut present = Vec::new();
    let mut missing = Vec::new();
    let mut seen = std::collections::HashSet::with_capacity(hashes.len());
    for h in hashes {
        validate_sha256(h)?;
        if !seen.insert(h.as_str()) {
            continue;
        }
        let p = blob_path(data_dir, h)?;
        if p.is_file() {
            present.push(h.clone());
        } else {
            missing.push(h.clone());
        }
    }
    Ok((present, missing))
}

/// Decompresses a `zstd` (or raw, if `is_zstd == false`) reader, verifies that
/// the decompressed bytes hash to `expected_sha256`, and atomically writes the
/// blob into the cache. Returns the decompressed byte size.
pub fn store_blob_from_reader<R: Read>(
    data_dir: &Path,
    expected_sha256: &str,
    reader: R,
    is_zstd: bool,
) -> Result<u64> {
    validate_sha256(expected_sha256)?;
    let target = blob_path(data_dir, expected_sha256)?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).context("creating blob shard directory")?;
    }
    if target.is_file() {
        let size = fs::metadata(&target)?.len();
        return Ok(size);
    }

    let tmp_dir = blobs_dir(data_dir).join(".tmp");
    fs::create_dir_all(&tmp_dir).context("creating blob temp directory")?;
    let mut tmp = tempfile::NamedTempFile::new_in(&tmp_dir).context("creating temp blob file")?;

    let mut hasher = Sha256::new();
    let mut total: u64 = 0;

    let copy_verified = |src: &mut dyn Read,
                         dst: &mut File,
                         hasher: &mut Sha256,
                         total: &mut u64|
     -> Result<()> {
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = src.read(&mut buf).context("reading uploaded blob")?;
            if n == 0 {
                break;
            }
            *total = total.saturating_add(n as u64);
            if *total > MAX_BLOB_BYTES {
                bail!("uploaded blob exceeds maximum size of {} MiB", MAX_BLOB_BYTES / (1024 * 1024));
            }
            hasher.update(&buf[..n]);
            dst.write_all(&buf[..n]).context("writing blob to disk")?;
        }
        Ok(())
    };

    if is_zstd {
        let mut decoder = zstd::stream::read::Decoder::new(reader)
            .context("initializing zstd decoder for blob")?;
        copy_verified(&mut decoder, tmp.as_file_mut(), &mut hasher, &mut total)?;
    } else {
        let mut r = reader;
        copy_verified(&mut r, tmp.as_file_mut(), &mut hasher, &mut total)?;
    }

    tmp.as_file_mut().flush()?;
    let actual = hex::encode(hasher.finalize());
    if actual != expected_sha256 {
        bail!("blob checksum mismatch: expected {expected_sha256}, got {actual}");
    }

    match tmp.persist_noclobber(&target) {
        Ok(_) => Ok(total),
        Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(total),
        Err(e) => Err(e.error).context("persisting blob into cache"),
    }
}

/// Unpacks a `zstd`-compressed delta tarball whose entries are named `<sha256>`.
/// Every entry is verified against its filename digest before persisting.
pub fn store_blob_batch_zstd<R: Read>(data_dir: &Path, reader: R) -> Result<(usize, u64)> {
    let decoder = zstd::stream::read::Decoder::new(reader)
        .context("initializing zstd decoder for delta batch")?;
    let mut archive = tar::Archive::new(decoder);
    let mut stored = 0usize;
    let mut total_bytes = 0u64;

    for entry in archive.entries().context("reading delta tarball entries")? {
        let entry = entry.context("reading delta tar entry")?;
        if !entry.header().entry_type().is_file() {
            bail!("delta batch archive may only contain regular files");
        }
        let name = {
            let path = entry.path().context("reading delta entry path")?;
            path.to_str()
                .ok_or_else(|| anyhow!("delta entry name is not valid UTF-8"))?
                .trim_start_matches("./")
                .to_string()
        };
        validate_sha256(&name)?;
        let size = entry.size();
        total_bytes = total_bytes.saturating_add(size);
        if total_bytes > MAX_BATCH_BYTES {
            bail!(
                "delta upload exceeds maximum batch size of {} MiB",
                MAX_BATCH_BYTES / (1024 * 1024)
            );
        }
        let written = store_blob_from_reader(data_dir, &name, entry, false)?;
        stored += 1;
        let _ = written;
    }
    Ok((stored, total_bytes))
}

/// Validates a relative POSIX file path inside a build context (no leading `/`, no `..`, no NUL).
pub fn validate_context_rel_path(path: &str) -> Result<PathBuf> {
    if path.is_empty() || path.len() > 512 || path.contains('\0') || path.contains('\\') {
        bail!("invalid context file path: {path:?}");
    }
    let p = Path::new(path);
    let mut clean = PathBuf::new();
    for c in p.components() {
        match c {
            Component::Normal(seg) => {
                if seg == ".git" && clean.as_os_str().is_empty() {
                    bail!("context path cannot include .git directory");
                }
                clean.push(seg);
            }
            Component::CurDir => {}
            _ => bail!("context file path must be relative with no `..`: {path}"),
        }
    }
    if clean.as_os_str().is_empty() {
        bail!("context file path cannot be empty");
    }
    Ok(clean)
}

/// Materializes a staged source context directory from `files` backed by the blob cache.
/// Returns the directory path (caller/deploy pipeline removes it after building).
pub fn materialize_context(
    data_dir: &Path,
    staging_id: &str,
    files: &[ContextFileEntry],
) -> Result<PathBuf> {
    if files.is_empty() {
        bail!("cannot deploy an empty directory");
    }
    if files.len() > MAX_CONTEXT_FILES {
        bail!("context contains more than {MAX_CONTEXT_FILES} files");
    }
    let root = contexts_dir(data_dir).join(format!("ctx-{staging_id}"));
    if root.exists() {
        let _ = fs::remove_dir_all(&root);
    }
    fs::create_dir_all(&root).context("creating staged context directory")?;

    let build_res: Result<()> = (|| {
        for entry in files {
            let rel = validate_context_rel_path(&entry.path)?;
            let blob = blob_path(data_dir, &entry.sha256)?;
            if !blob.is_file() {
                bail!("missing uploaded blob {} for {}", entry.sha256, entry.path);
            }
            let dest = root.join(&rel);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("creating parent directory for {}", entry.path))?;
            }
            // Hardlink when possible (zero-copy!), falling back to copy across filesystems.
            if fs::hard_link(&blob, &dest).is_err() {
                fs::copy(&blob, &dest)
                    .with_context(|| format!("materializing {}", entry.path))?;
            } else {
                // If hard-linked, avoid mutating mode bits on the shared cache blob;
                // copy when executable bit differs from the cached file.
                let want_exec = (entry.mode & 0o111) != 0;
                let has_exec = (fs::metadata(&dest)?.permissions().mode() & 0o111) != 0;
                if want_exec != has_exec {
                    fs::remove_file(&dest)?;
                    fs::copy(&blob, &dest)?;
                }
            }
            let mode = if (entry.mode & 0o111) != 0 {
                0o755
            } else {
                0o644
            };
            let _ = fs::set_permissions(&dest, fs::Permissions::from_mode(mode));
        }
        Ok(())
    })();

    if let Err(e) = build_res {
        let _ = fs::remove_dir_all(&root);
        return Err(e);
    }
    Ok(root)
}

/// Reassembles a Docker/OCI image tarball from content-addressed blob entries
/// and loads it into the container engine.
pub async fn load_reassembled_image(
    data_dir: &Path,
    runtime: &dyn Runtime,
    entries: &[ArchiveBlobEntry],
    log: &LogSink,
) -> Result<()> {
    if entries.is_empty() {
        bail!("image archive has no entries");
    }
    let data_dir_owned = data_dir.to_path_buf();
    let entries_owned = entries.to_vec();

    let tmp_tar = tokio::task::spawn_blocking(move || -> Result<tempfile::NamedTempFile> {
        let work_dir = contexts_dir(&data_dir_owned);
        fs::create_dir_all(&work_dir).context("creating image reassembly directory")?;
        let tmp = tempfile::Builder::new()
            .prefix("img-")
            .suffix(".tar")
            .tempfile_in(&work_dir)
            .context("creating temp image tar")?;
        let mut builder = tar::Builder::new(tmp);
        for entry in &entries_owned {
            let rel = validate_context_rel_path(&entry.archive_path)?;
            let blob = blob_path(&data_dir_owned, &entry.sha256)?;
            if !blob.is_file() {
                bail!(
                    "missing uploaded image layer/blob {} ({})",
                    entry.sha256,
                    entry.archive_path
                );
            }
            let mut f = File::open(&blob)
                .with_context(|| format!("opening blob {}", entry.sha256))?;
            let meta = f.metadata()?;
            let mut header = tar::Header::new_gnu();
            header.set_size(meta.len());
            header.set_mode(if entry.mode == 0 { 0o644 } else { entry.mode & 0o777 });
            header.set_cksum();
            builder
                .append_data(&mut header, &rel, &mut f)
                .with_context(|| format!("appending {} to image archive", entry.archive_path))?;
        }
        let mut tmp = builder.into_inner().context("finalizing image archive")?;
        tmp.flush()?;
        Ok(tmp)
    })
    .await
    .context("reassembling image tar")??;

    runtime.load_image(tmp_tar.path(), log).await
}

// ------------------------------------------------ Direct Linux Binary Wrapping

/// Result of inspecting an uploaded Linux ELF binary header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElfInfo {
    /// Target OCI architecture (`amd64`, `arm64`, `386`, `arm`, `riscv64`).
    pub arch: &'static str,
    /// True when the ELF has no `PT_INTERP` program header (statically linked, e.g. musl).
    pub is_static: bool,
}

/// Validates ELF magic and extracts architecture + static/dynamic linkage from
/// the ELF header and program headers.
pub fn inspect_elf(bytes: &[u8]) -> Result<ElfInfo> {
    if bytes.len() < 64 || &bytes[0..4] != b"\x7fELF" {
        bail!(
            "the uploaded file is not a Linux ELF binary (missing ELF magic). \
             Cross-compile for Linux first (e.g. `cargo build --release --target x86_64-unknown-linux-musl` \
             or `GOOS=linux GOARCH=amd64 go build`)"
        );
    }
    let class = bytes[4]; // 1 = 32-bit, 2 = 64-bit
    let endian = bytes[5]; // 1 = little-endian, 2 = big-endian
    if endian != 1 {
        bail!("only little-endian Linux ELF binaries are supported");
    }
    let e_machine = u16::from_le_bytes([bytes[18], bytes[19]]);
    let arch = match e_machine {
        62 => "amd64",  // EM_X86_64
        183 => "arm64", // EM_AARCH64
        3 => "386",     // EM_386
        40 => "arm",    // EM_ARM
        243 => "riscv64",
        other => bail!("unsupported ELF CPU architecture (e_machine={other})"),
    };

    // Walk program headers looking for PT_INTERP (type == 3).
    let (phoff, phentsize, phnum) = match class {
        2 => {
            let phoff = u64::from_le_bytes(bytes[32..40].try_into().unwrap()) as usize;
            let phentsize = u16::from_le_bytes([bytes[54], bytes[55]]) as usize;
            let phnum = u16::from_le_bytes([bytes[56], bytes[57]]) as usize;
            (phoff, phentsize, phnum)
        }
        1 => {
            let phoff = u32::from_le_bytes(bytes[28..32].try_into().unwrap()) as usize;
            let phentsize = u16::from_le_bytes([bytes[42], bytes[43]]) as usize;
            let phnum = u16::from_le_bytes([bytes[44], bytes[45]]) as usize;
            (phoff, phentsize, phnum)
        }
        _ => bail!("invalid ELF class {class}"),
    };

    let mut has_interp = false;
    if phentsize >= 4 && phnum > 0 {
        for i in 0..phnum.min(128) {
            let off = match phoff.checked_add(i.saturating_mul(phentsize)) {
                Some(o) if o + 4 <= bytes.len() => o,
                _ => break,
            };
            let p_type = u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
            if p_type == 3 {
                // PT_INTERP
                has_interp = true;
                break;
            }
        }
    }

    Ok(ElfInfo {
        arch,
        is_static: !has_interp,
    })
}

/// Builds a pure-Rust `FROM scratch` Docker/OCI load tarball wrapping a static
/// Linux binary at `/app` with mode `0755`, zero Dockerfile and zero network pull.
pub fn build_scratch_binary_tar(
    binary_path: &Path,
    tag: &str,
    arch: &str,
    port: u16,
    args: &[String],
    out_tar: &Path,
) -> Result<()> {
    // 1. Build the single uncompressed layer tar containing `app` (0755).
    let mut layer_bytes = Vec::new();
    {
        let mut layer = tar::Builder::new(&mut layer_bytes);
        let mut bin = File::open(binary_path).context("opening binary blob")?;
        let meta = bin.metadata()?;
        let mut hdr = tar::Header::new_gnu();
        hdr.set_size(meta.len());
        hdr.set_mode(0o755);
        hdr.set_uid(0);
        hdr.set_gid(0);
        hdr.set_mtime(0);
        hdr.set_cksum();
        layer.append_data(&mut hdr, "app", &mut bin)?;
        layer.finish()?;
    }

    let layer_sha = hex::encode(Sha256::digest(&layer_bytes));
    let layer_path = format!("{layer_sha}/layer.tar");

    // 2. Construct OCI/Docker image config JSON.
    let mut exposed = serde_json::Map::new();
    exposed.insert(format!("{port}/tcp"), json!({}));
    let mut cfg = json!({
        "architecture": arch,
        "os": "linux",
        "created": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "config": {
            "Entrypoint": ["/app"],
            "Env": [
                "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
                format!("PORT={port}")
            ],
            "ExposedPorts": exposed,
            "WorkingDir": "/",
        },
        "rootfs": {
            "type": "layers",
            "diff_ids": [format!("sha256:{layer_sha}")],
        },
        "history": [
            {
                "created_by": "rig up --binary (FROM scratch)",
            }
        ],
    });
    if !args.is_empty() {
        cfg["config"]["Cmd"] = json!(args);
    }
    let config_bytes = serde_json::to_vec(&cfg)?;
    let config_sha = hex::encode(Sha256::digest(&config_bytes));
    let config_name = format!("{config_sha}.json");

    // 3. Construct Docker `manifest.json` (accepted by both `docker load` and `podman load`).
    let manifest = json!([
        {
            "Config": config_name,
            "RepoTags": [tag],
            "Layers": [layer_path],
        }
    ]);
    let manifest_bytes = serde_json::to_vec(&manifest)?;

    // 4. Write the final image tar archive.
    let out_file = File::create(out_tar).context("creating scratch image tar")?;
    let mut outer = tar::Builder::new(out_file);

    let append_mem = |builder: &mut tar::Builder<File>, name: &str, data: &[u8]| -> Result<()> {
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        h.set_mtime(0);
        h.set_cksum();
        builder.append_data(&mut h, name, data)?;
        Ok(())
    };

    append_mem(&mut outer, &config_name, &config_bytes)?;
    append_mem(&mut outer, &layer_path, &layer_bytes)?;
    append_mem(&mut outer, "manifest.json", &manifest_bytes)?;
    outer.finish()?;
    Ok(())
}

/// Wraps an uploaded Linux binary blob into a ready-to-run OCI image in `runtime`.
///
/// - Statically linked ELF binaries (no `PT_INTERP`) with no custom `base_image`
///   are packaged in pure Rust into a minimal `FROM scratch` OCI tar and loaded
///   directly via `Runtime::load_image` (zero network pull, zero Dockerfile).
/// - Dynamically linked ELF binaries (or when `base_image` is specified) are
///   wrapped with a generated ephemeral `Dockerfile` on `distroless` (or the
///   requested base image) via `Runtime::build`.
#[allow(clippy::too_many_arguments)]
pub async fn wrap_binary_into_image(
    data_dir: &Path,
    runtime: &dyn Runtime,
    app_id: &str,
    blob_sha256: &str,
    tag: &str,
    port: u16,
    args: &[String],
    base_image: Option<&str>,
    log: &LogSink,
) -> Result<ElfInfo> {
    let bin_blob = blob_path(data_dir, blob_sha256)?;
    if !bin_blob.is_file() {
        bail!("uploaded binary blob {blob_sha256} not found in server cache");
    }

    // Read ELF header + program headers (first 16 KiB is more than enough).
    let bin_for_elf = bin_blob.clone();
    let elf = tokio::task::spawn_blocking(move || -> Result<ElfInfo> {
        let mut f = File::open(&bin_for_elf)?;
        let mut head = vec![0u8; 16 * 1024];
        let n = f.read(&mut head)?;
        head.truncate(n);
        inspect_elf(&head)
    })
    .await
    .context("inspecting uploaded binary")??;

    if elf.is_static && base_image.is_none() {
        log.line(
            "build",
            format!(
                "Detected statically linked linux/{} ELF binary — wrapping in minimal FROM scratch image",
                elf.arch
            ),
        )
        .await;
        let data_dir_owned = data_dir.to_path_buf();
        let tag_owned = tag.to_string();
        let args_owned = args.to_vec();
        let arch = elf.arch;
        let tmp_tar = tokio::task::spawn_blocking(move || -> Result<tempfile::NamedTempFile> {
            let work_dir = contexts_dir(&data_dir_owned);
            fs::create_dir_all(&work_dir)?;
            let tmp = tempfile::Builder::new()
                .prefix("bin-img-")
                .suffix(".tar")
                .tempfile_in(&work_dir)?;
            build_scratch_binary_tar(&bin_blob, &tag_owned, arch, port, &args_owned, tmp.path())?;
            Ok(tmp)
        })
        .await
        .context("wrapping static binary into scratch image")??;
        runtime.load_image(tmp_tar.path(), log).await?;
        return Ok(elf);
    }

    let base = base_image.unwrap_or(DEFAULT_DYNAMIC_BASE_IMAGE);
    log.line(
        "build",
        format!(
            "Detected {} linux/{} ELF binary — wrapping on {base}",
            if elf.is_static { "static" } else { "dynamically linked" },
            elf.arch
        ),
    )
    .await;

    let work_dir = contexts_dir(data_dir);
    tokio::fs::create_dir_all(&work_dir).await?;
    let ctx_dir = tempfile::Builder::new()
        .prefix("bin-ctx-")
        .tempdir_in(&work_dir)?;
    let bin_dest = ctx_dir.path().join("app");
    tokio::fs::copy(&bin_blob, &bin_dest).await?;
    let _ = fs::set_permissions(&bin_dest, fs::Permissions::from_mode(0o755));

    let mut dockerfile = format!(
        "FROM {base}\nCOPY app /app\nEXPOSE {port}\nENV PORT={port}\nENTRYPOINT [\"/app\"]\n"
    );
    if !args.is_empty() {
        dockerfile.push_str(&format!("CMD {}\n", serde_json::to_string(args)?));
    }
    tokio::fs::write(ctx_dir.path().join("Dockerfile"), dockerfile).await?;

    runtime
        .build(
            BuildSpec {
                context: ctx_dir.path().to_path_buf(),
                dockerfile: "Dockerfile".into(),
                tag: tag.to_string(),
                labels: HashMap::from([
                    (LABEL_MANAGED.to_string(), "true".to_string()),
                    (LABEL_APP.to_string(), app_id.to_string()),
                ]),
            },
            log,
        )
        .await?;

    Ok(elf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_elf64(e_machine: u16, include_pt_interp: bool) -> Vec<u8> {
        let mut buf = vec![0u8; 256];
        buf[0..4].copy_from_slice(b"\x7fELF");
        buf[4] = 2; // ELFCLASS64
        buf[5] = 1; // ELFDATA2LSB
        buf[18..20].copy_from_slice(&e_machine.to_le_bytes());
        buf[32..40].copy_from_slice(&64u64.to_le_bytes()); // e_phoff = 64
        buf[54..56].copy_from_slice(&56u16.to_le_bytes()); // e_phentsize = 56
        buf[56..58].copy_from_slice(&2u16.to_le_bytes()); // e_phnum = 2
        // Program header 0: PT_LOAD (1)
        buf[64..68].copy_from_slice(&1u32.to_le_bytes());
        // Program header 1: PT_INTERP (3) or PT_NOTE (4)
        let second = if include_pt_interp { 3u32 } else { 4u32 };
        buf[120..124].copy_from_slice(&second.to_le_bytes());
        buf
    }

    #[test]
    fn elf_inspection_distinguishes_static_and_dynamic() {
        let static_amd64 = fake_elf64(62, false);
        assert_eq!(
            inspect_elf(&static_amd64).unwrap(),
            ElfInfo {
                arch: "amd64",
                is_static: true
            }
        );

        let dynamic_arm64 = fake_elf64(183, true);
        assert_eq!(
            inspect_elf(&dynamic_arm64).unwrap(),
            ElfInfo {
                arch: "arm64",
                is_static: false
            }
        );

        assert!(inspect_elf(b"#!/bin/sh\necho hi\n").is_err());
    }

    #[test]
    fn blob_store_zstd_dedup_and_checksum_verification() {
        let dir = tempfile::tempdir().unwrap();
        let payload = b"hello world from rig up blob store!";
        let digest = hex::encode(Sha256::digest(payload));

        let (present, missing) = check_blobs(dir.path(), std::slice::from_ref(&digest)).unwrap();
        assert!(present.is_empty());
        assert_eq!(missing, vec![digest.clone()]);

        let compressed = zstd::stream::encode_all(payload.as_slice(), 3).unwrap();
        let sz = store_blob_from_reader(dir.path(), &digest, compressed.as_slice(), true).unwrap();
        assert_eq!(sz, payload.len() as u64);

        let (present, missing) = check_blobs(dir.path(), std::slice::from_ref(&digest)).unwrap();
        assert_eq!(present, vec![digest.clone()]);
        assert!(missing.is_empty());

        // Mismatched checksum is rejected.
        let wrong = "0".repeat(64);
        assert!(store_blob_from_reader(dir.path(), &wrong, compressed.as_slice(), true).is_err());
    }

    #[test]
    fn batch_delta_and_context_materialization_work() {
        let dir = tempfile::tempdir().unwrap();
        let f1 = b"FROM scratch\n";
        let f2 = b"#!/bin/sh\necho ok\n";
        let h1 = hex::encode(Sha256::digest(f1));
        let h2 = hex::encode(Sha256::digest(f2));

        let mut tar_bytes = Vec::new();
        {
            let mut b = tar::Builder::new(&mut tar_bytes);
            for (h, data) in [(&h1, f1.as_slice()), (&h2, f2.as_slice())] {
                let mut hdr = tar::Header::new_gnu();
                hdr.set_size(data.len() as u64);
                hdr.set_mode(0o644);
                hdr.set_cksum();
                b.append_data(&mut hdr, h, data).unwrap();
            }
            b.finish().unwrap();
        }
        let zst = zstd::stream::encode_all(tar_bytes.as_slice(), 3).unwrap();
        let (stored, _) = store_blob_batch_zstd(dir.path(), zst.as_slice()).unwrap();
        assert_eq!(stored, 2);

        let ctx = materialize_context(
            dir.path(),
            "test-1",
            &[
                ContextFileEntry {
                    path: "Dockerfile".into(),
                    sha256: h1,
                    mode: 0o644,
                    size: f1.len() as u64,
                },
                ContextFileEntry {
                    path: "scripts/run.sh".into(),
                    sha256: h2,
                    mode: 0o755,
                    size: f2.len() as u64,
                },
            ],
        )
        .unwrap();

        assert_eq!(fs::read(ctx.join("Dockerfile")).unwrap(), f1);
        assert_eq!(fs::read(ctx.join("scripts/run.sh")).unwrap(), f2);
        let run_mode = fs::metadata(ctx.join("scripts/run.sh"))
            .unwrap()
            .permissions()
            .mode();
        assert_ne!(run_mode & 0o111, 0, "executable bit should be preserved");

        // Path traversal is rejected.
        assert!(validate_context_rel_path("../etc/passwd").is_err());
        assert!(validate_context_rel_path("/etc/passwd").is_err());
        assert!(validate_context_rel_path(".git/config").is_err());
    }

    #[test]
    fn scratch_binary_tar_contains_valid_manifest_config_and_executable() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("mybin");
        fs::write(&bin, fake_elf64(62, false)).unwrap();
        let tar_out = dir.path().join("out.tar");

        build_scratch_binary_tar(
            &bin,
            "rig/demo:up-1234",
            "amd64",
            8080,
            &["--serve".into()],
            &tar_out,
        )
        .unwrap();

        let mut archive = tar::Archive::new(File::open(&tar_out).unwrap());
        let mut names = Vec::new();
        for entry in archive.entries().unwrap() {
            let e = entry.unwrap();
            names.push(e.path().unwrap().to_string_lossy().into_owned());
        }
        assert!(names.iter().any(|n| n == "manifest.json"));
        assert!(names.iter().any(|n| n.ends_with(".json") && n != "manifest.json"));
        assert!(names.iter().any(|n| n.ends_with("/layer.tar")));
    }
}
