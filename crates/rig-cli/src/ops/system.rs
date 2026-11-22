//! System, project, and database operations shared between CLI and MCP.

use rig_api::{Me, SystemInfo};

use crate::api::Client;
use crate::error::CliResult;

pub struct StatusSnapshot {
    pub me: Me,
    pub system: SystemInfo,
}

/// Fetches current identity and server health concurrently in a single round-trip window.
pub async fn status(api: &Client) -> CliResult<StatusSnapshot> {
    let (me, system) = tokio::try_join!(api.me(), api.system())?;
    Ok(StatusSnapshot { me, system })
}
