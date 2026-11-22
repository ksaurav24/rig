//! Shared domain operations used by both the CLI commands and `rig mcp`.
//!
//! Keeping resolution, filtering, streaming and mutation logic here prevents
//! behavior from drifting between interactive CLI invocations and MCP tool calls.

pub mod apps;
pub mod deployments;
pub mod env;
pub mod system;
