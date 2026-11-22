//! Binary entrypoint for `rig`.

use clap::Parser;
use rig_cli::{Cli, run};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let cli = Cli::parse();
    if let Err(err) = run(cli).await {
        if err.is_detached() {
            eprintln!("{err}");
        } else {
            eprintln!("rig: {err}");
        }
        std::process::exit(err.exit_code());
    }
}
