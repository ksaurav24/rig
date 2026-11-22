//! `rig mcp`: a Model Context Protocol server on stdio.
//!
//! Speaks line-delimited JSON-RPC 2.0. Stdout carries protocol messages and
//! nothing else, so every diagnostic goes to stderr. Tools call the same HTTP
//! API and shared `crate::ops` logic as the CLI, with the context's API token,
//! so an agent can never do more than that token's role allows. There is
//! deliberately no tool that reveals secret values.

pub mod protocol;
pub mod server;
pub mod tools;

pub use server::{ClientSource, serve};
