//! Native Rust container terminal WebSocket bridge (§9.3).
//!
//! Opens authenticated WebSockets to `/api/v1/apps/{id}/terminal` on demand
//! using `rig-client` (with OS keychain bearer token + pinned TLS cert) and
//! streams binary PTY chunks to xterm.js over a Tauri `Channel`.

use std::collections::HashMap;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use rig_client::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, mpsc};
use tokio_tungstenite::tungstenite::Message;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TerminalFrame {
    Output { data: Vec<u8> },
    State { state: String, message: Option<String> },
}

enum TerminalCmd {
    Input(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Close,
}

#[derive(Default)]
pub struct TerminalManager {
    sessions: RwLock<HashMap<String, mpsc::Sender<TerminalCmd>>>,
}

impl TerminalManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub async fn open(
        self: &Arc<Self>,
        session_id: String,
        client: Client,
        app_id: String,
        cols: u16,
        rows: u16,
        mut emit: impl FnMut(TerminalFrame) -> bool + Send + 'static,
    ) -> Result<(), String> {
        self.close(&session_id).await;
        let ws = client
            .connect_terminal_ws(&app_id, cols, rows)
            .await
            .map_err(|e| e.to_string())?;

        let (tx, mut rx) = mpsc::channel::<TerminalCmd>(256);
        self.sessions
            .write()
            .await
            .insert(session_id.clone(), tx);

        let mgr = Arc::clone(self);
        tokio::spawn(async move {
            let (mut sink, mut stream) = ws.split();
            let _ = emit(TerminalFrame::State {
                state: "connected".into(),
                message: None,
            });

            loop {
                tokio::select! {
                    cmd = rx.recv() => {
                        match cmd {
                            Some(TerminalCmd::Input(bytes)) => {
                                if sink.send(Message::Binary(bytes.into())).await.is_err() {
                                    break;
                                }
                            }
                            Some(TerminalCmd::Resize { cols, rows }) => {
                                let payload = serde_json::json!({
                                    "type": "resize",
                                    "cols": cols,
                                    "rows": rows,
                                }).to_string();
                                if sink.send(Message::Text(payload.into())).await.is_err() {
                                    break;
                                }
                            }
                            Some(TerminalCmd::Close) | None => {
                                let _ = sink.send(Message::Close(None)).await;
                                break;
                            }
                        }
                    }
                    incoming = stream.next() => {
                        match incoming {
                            Some(Ok(Message::Binary(bytes))) => {
                                if !emit(TerminalFrame::Output { data: bytes.to_vec() }) {
                                    break;
                                }
                            }
                            Some(Ok(Message::Text(txt))) => {
                                if !emit(TerminalFrame::Output { data: txt.as_bytes().to_vec() }) {
                                    break;
                                }
                            }
                            Some(Ok(Message::Close(_))) | None => break,
                            Some(Ok(_)) => {}
                            Some(Err(e)) => {
                                let _ = emit(TerminalFrame::State {
                                    state: "error".into(),
                                    message: Some(e.to_string()),
                                });
                                break;
                            }
                        }
                    }
                }
            }

            let _ = emit(TerminalFrame::State {
                state: "closed".into(),
                message: None,
            });
            mgr.sessions.write().await.remove(&session_id);
        });

        Ok(())
    }

    pub async fn write(&self, session_id: &str, data: Vec<u8>) -> Result<(), String> {
        let tx = self.sessions.read().await.get(session_id).cloned();
        let Some(tx) = tx else {
            return Err("terminal session is not open".into());
        };
        tx.send(TerminalCmd::Input(data))
            .await
            .map_err(|_| "terminal session closed".to_string())
    }

    pub async fn resize(&self, session_id: &str, cols: u16, rows: u16) -> Result<(), String> {
        let tx = self.sessions.read().await.get(session_id).cloned();
        let Some(tx) = tx else {
            return Ok(());
        };
        let _ = tx.send(TerminalCmd::Resize { cols, rows }).await;
        Ok(())
    }

    pub async fn close(&self, session_id: &str) {
        if let Some(tx) = self.sessions.write().await.remove(session_id) {
            let _ = tx.send(TerminalCmd::Close).await;
        }
    }

    pub async fn close_all(&self) {
        let mut sessions = self.sessions.write().await;
        for (_, tx) in sessions.drain() {
            let _ = tx.send(TerminalCmd::Close).await;
        }
    }
}
