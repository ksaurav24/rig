//! Concurrent stdio transport loop for `rig mcp`.
//!
//! - Starts even when no context is configured (`initialize`, `ping`, `tools/list`
//!   always work; `tools/call` reports an actionable `isError` message).
//! - Spawns each `tools/call` on its own Tokio task so a long-running `deploy_app`
//!   never blocks `ping`, `tools/list`, or other tool calls.
//! - Supports `notifications/cancelled` via an active `AbortHandle` registry.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use crate::api::Client;
use crate::config::{Config, ResolveOptions};
use crate::error::CliResult;

use super::protocol::{INVALID_REQUEST, MAX_LINE_BYTES, Step, dispatch, error, tool_result};
use super::tools::{Outcome, ProgressSink, run as run_tool};

/// Configuration overrides used to resolve the Rig API client per `tools/call`.
#[derive(Clone, Default)]
pub struct ClientSource {
    pub context: Option<String>,
    pub server: Option<String>,
    pub token: Option<String>,
    pub insecure: bool,
}

impl ClientSource {
    fn resolve(&self) -> CliResult<Client> {
        let config = Config::load()?;
        let opts = ResolveOptions {
            context: self.context.as_deref(),
            server: self.server.as_deref(),
            token: self.token.as_deref(),
            insecure: self.insecure,
        };
        let ctx = config.resolve_with(&opts)?;
        Client::with_options(ctx, false, false)
    }
}

fn id_key(id: &Value) -> String {
    serde_json::to_string(id).unwrap_or_default()
}

pub async fn serve(source: ClientSource) -> CliResult<()> {
    let (tx, mut rx) = mpsc::channel::<Value>(128);

    // Single writer task guarantees stdout lines never interleave across concurrent tool calls.
    let writer = tokio::spawn(async move {
        let mut out = tokio::io::stdout();
        while let Some(msg) = rx.recv().await {
            let Ok(mut bytes) = serde_json::to_vec(&msg) else {
                continue;
            };
            bytes.push(b'\n');
            if out.write_all(&bytes).await.is_err() {
                break;
            }
            if out.flush().await.is_err() {
                break;
            }
        }
    });

    let active: Arc<Mutex<HashMap<String, AbortHandle>>> = Arc::new(Mutex::new(HashMap::new()));
    let mut lines = BufReader::new(tokio::io::stdin()).lines();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        if line.len() > MAX_LINE_BYTES {
            let _ = tx
                .send(error(Value::Null, INVALID_REQUEST, "request is too large"))
                .await;
            continue;
        }
        match dispatch(&line) {
            Step::Silent => {}
            Step::Reply(reply) => {
                let _ = tx.send(reply).await;
            }
            Step::Cancel { request_id } => {
                let key = id_key(&request_id);
                if let Some(handle) = active.lock().expect("lock").remove(&key) {
                    handle.abort();
                }
            }
            Step::Call {
                id,
                tool,
                args,
                progress_token,
            } => {
                let key = id_key(&id);
                let tx_task = tx.clone();
                let source_task = source.clone();
                let active_task = Arc::clone(&active);

                // Register under lock before spawning so a fast Cancel never races.
                let mut guard = active.lock().expect("lock");
                let key_inner = key.clone();
                let join = tokio::spawn(async move {
                    let progress = progress_token.map(|tok| ProgressSink::new(tok, tx_task.clone()));
                    let (payload, is_error) = match source_task.resolve() {
                        Ok(api) => match run_tool(&api, tool.name, &args, progress).await {
                            Ok(Outcome { value, failed }) => (value, failed),
                            Err(e) => (Value::String(e.to_string()), true),
                        },
                        Err(e) => (Value::String(e.to_string()), true),
                    };

                    // Only send the reply if the request was not already cancelled.
                    let still_active = active_task
                        .lock()
                        .expect("lock")
                        .remove(&key_inner)
                        .is_some();
                    if still_active {
                        let _ = tx_task.send(tool_result(id, payload, is_error)).await;
                    }
                });
                guard.insert(key, join.abort_handle());
            }
        }
    }

    // Drop the reader's sender; once all spawned tasks finish and drop their
    // clones, the writer drains the final message and exits cleanly.
    drop(tx);
    let _ = writer.await;
    Ok(())
}
