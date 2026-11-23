//! Pre-compresses the built dashboard in `web/dist` into `OUT_DIR` and emits
//! a sorted static asset table (`dist_assets.rs`) with precomputed ETags, MIME
//! types, and Brotli/Gzip payloads.
//!
//! Storing compressible JS/CSS/HTML/SVG assets solely as `.br` + `.gz` both
//! shrinks the final `rig-server` binary and eliminates on-the-fly compression
//! CPU cost on every browser request.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use flate2::{Compression, write::GzEncoder};
use sha2::{Digest, Sha256};

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dist = manifest_dir.join("../../web/dist");
    fs::create_dir_all(&dist).expect("create web/dist");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=migrations");
    println!("cargo:rerun-if-changed={}", dist.display());

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let asset_out = out_dir.join("dist_compressed");
    let _ = fs::remove_dir_all(&asset_out);
    fs::create_dir_all(&asset_out).expect("create dist_compressed");

    let mut files = Vec::new();
    collect_files(&dist, &dist, &mut files);
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let mut code = format!("static ASSETS: [Asset; {}] = [\n", files.len());
    for (idx, (rel_path, abs_path)) in files.iter().enumerate() {
        println!("cargo:rerun-if-changed={}", abs_path.display());
        let raw = fs::read(abs_path).expect("read web/dist asset");
        let digest = Sha256::digest(&raw);
        let etag = format!("\"{}\"", hex::encode(&digest[..12]));
        let mime = mime_guess::from_path(rel_path)
            .first_or_octet_stream()
            .essence_str()
            .to_owned();
        let is_html = mime == "text/html";
        let immutable = rel_path.starts_with("assets/");

        let data_expr = if should_compress(rel_path, raw.len()) {
            let gz = compress_gzip(&raw);
            let br = compress_brotli(&raw);
            if gz.len() < raw.len() && br.len() < raw.len() {
                let br_path = asset_out.join(format!("{idx}.br"));
                let gz_path = asset_out.join(format!("{idx}.gz"));
                fs::write(&br_path, &br).expect("write .br asset");
                fs::write(&gz_path, &gz).expect("write .gz asset");
                format!(
                    "AssetData::Compressed {{ br: include_bytes!({:?}), gz: include_bytes!({:?}) }}",
                    br_path.to_string_lossy(),
                    gz_path.to_string_lossy(),
                )
            } else {
                format!("AssetData::Raw(include_bytes!({:?}))", abs_path.to_string_lossy())
            }
        } else {
            format!("AssetData::Raw(include_bytes!({:?}))", abs_path.to_string_lossy())
        };

        code.push_str(&format!(
            "    Asset {{ path: {:?}, mime: {:?}, etag: {:?}, is_html: {}, immutable: {}, data: {}, identity: std::sync::OnceLock::new() }},\n",
            rel_path, mime, etag, is_html, immutable, data_expr
        ));
    }
    code.push_str("];\n");
    fs::write(out_dir.join("dist_assets.rs"), code).expect("write dist_assets.rs");
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            println!("cargo:rerun-if-changed={}", path.display());
            collect_files(root, &path, out);
        } else if meta.is_file() {
            let Ok(rel) = path.strip_prefix(root) else {
                continue;
            };
            let rel_str = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            out.push((rel_str, path));
        }
    }
}

fn should_compress(rel_path: &str, len: usize) -> bool {
    if len < 256 {
        return false;
    }
    let ext = Path::new(rel_path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    matches!(
        ext.as_deref(),
        Some("js" | "mjs" | "css" | "html" | "svg" | "json" | "txt" | "xml" | "wasm" | "map")
    )
}

fn compress_gzip(raw: &[u8]) -> Vec<u8> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::best());
    enc.write_all(raw).expect("gzip compress");
    enc.finish().expect("gzip finish")
}

fn compress_brotli(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = brotli::CompressorWriter::new(&mut out, 4096, 6, 22);
        enc.write_all(raw).expect("brotli compress");
    }
    out
}
