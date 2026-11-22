//! The dashboard is embedded from `web/dist`. Make sure the folder exists so
//! the server still compiles before the dashboard has been built; it then
//! serves a short note explaining how to build it.

fn main() {
    let dist = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/dist");
    std::fs::create_dir_all(&dist).expect("create web/dist");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=migrations");
    println!("cargo:rerun-if-changed=../../web/dist");
}
