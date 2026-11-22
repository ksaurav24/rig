//! `rig databases`.

use crate::api::Client;
use crate::error::CliResult;
use crate::output::{label, print_json, table};

pub async fn run(api: &Client, json_out: bool, wide: bool) -> CliResult<()> {
    let dbs = api.list_databases().await?;
    if json_out {
        return print_json(&dbs);
    }
    let rows = dbs
        .iter()
        .map(|d| {
            vec![
                format!("{}/{}", d.project_name, d.name),
                format!("{} {}", label(&d.engine), d.version),
                label(&d.status),
                format!("{}:{}", d.host, d.port),
            ]
        })
        .collect();
    table(
        &["DATABASE", "ENGINE", "STATUS", "ADDRESS IN PROJECT"],
        rows,
        Some("No databases yet."),
        wide,
    );
    Ok(())
}
