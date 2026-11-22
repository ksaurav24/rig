//! `rig status`.

use serde_json::json;

use crate::api::Client;
use crate::error::CliResult;
use crate::ops;
use crate::output::{bytes, label, print_json};

pub async fn run(api: &Client, json_out: bool) -> CliResult<()> {
    let snap = ops::system::status(api).await?;
    if json_out {
        return print_json(&json!({
            "server": api.base(),
            "me": snap.me,
            "system": snap.system,
        }));
    }
    let me = snap.me;
    let sys = snap.system;
    println!(
        "{} ({}) in {}, as {}",
        me.user.name,
        me.user.email,
        me.org.name,
        label(&me.role)
    );
    println!("Server   {} · Rig {}", api.base(), sys.version);
    match (&sys.engine, &sys.engine_version) {
        (Some(engine), Some(version)) => println!("Engine   {engine} {version}"),
        _ => println!("Engine   not reachable"),
    }
    println!(
        "Proxy    {}",
        if sys.proxy_ready {
            "ready"
        } else {
            "not reachable"
        }
    );
    println!(
        "Apps     {} running of {}",
        sys.apps_running, sys.apps_total
    );
    println!(
        "Load     CPU {:.0}% · memory {} of {} · disk {} of {}",
        sys.cpu_percent,
        bytes(sys.memory_used),
        bytes(sys.memory_total),
        bytes(sys.disk_used),
        bytes(sys.disk_total)
    );
    Ok(())
}
