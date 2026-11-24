//! Per-server connection lifecycle manager (§5.4).
//!
//! - Every paired server maintains one lightweight `GET /api/v1/events` SSE
//!   subscription in Rust, driving left-rail health dots and OS notifications
//!   even when the server is inactive.
//! - Offline servers back off exponentially (1s -> 2s -> 4s -> 8s -> 15s -> 30s)
//!   and keep serving their cached read-only snapshot.
//! - Switching servers immediately closes the previous server's view streams
//!   (log streams & terminals) while preserving its cache.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use rig_api::ServerEvent;
use rig_client::{Client, ClientError, ClientOptions};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::{RwLock, watch};

use crate::keychain::TokenStore;
use crate::registry::{ServerEntry, ServerHealth, ServerRegistry};

const BACKOFF_SECS: [u64; 6] = [1, 2, 4, 8, 15, 30];
const MAX_RECENT_EVENTS: usize = 50;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerStatusPayload {
    pub server_id: String,
    pub health: ServerHealth,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerEventPayload {
    pub server_id: String,
    pub server_name: String,
    pub event: ServerEvent,
}

#[derive(Default)]
pub struct ConnectionManager {
    watchers: RwLock<HashMap<String, watch::Sender<bool>>>,
    health: RwLock<HashMap<String, (ServerHealth, Option<String>)>>,
    recent_events: RwLock<HashMap<String, VecDeque<ServerEvent>>>,
}

impl ConnectionManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub async fn health_map(&self) -> HashMap<String, (ServerHealth, Option<String>)> {
        self.health.read().await.clone()
    }

    pub async fn recent_events_for(&self, server_id: &str) -> Vec<ServerEvent> {
        self.recent_events
            .read()
            .await
            .get(server_id)
            .map(|q| q.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn build_client(entry: &ServerEntry, tokens: &TokenStore) -> Result<Client, String> {
        let token = tokens
            .get(&entry.id)
            .map_err(|e| format!("reading keychain token: {e}"))?
            .ok_or_else(|| "device token missing from keychain; re-pair this server".to_string())?;
        Client::build(ClientOptions {
            base_url: entry.url.clone(),
            token: Some(token),
            pinned_tls_sha256: entry.pinned_tls_sha256.clone(),
            user_agent: concat!("rig-desktop/", env!("CARGO_PKG_VERSION")).into(),
            verbose: false,
            warn_skew: false,
        })
        .map_err(|e| e.to_string())
    }

    /// Starts (or restarts) the lightweight `/api/v1/events` SSE loop for one server.
    pub async fn ensure_watcher(
        self: &Arc<Self>,
        app: AppHandle,
        entry: ServerEntry,
        tokens: Arc<TokenStore>,
        registry: Arc<RwLock<ServerRegistry>>,
    ) {
        self.stop_watcher(&entry.id).await;
        let (tx, mut stop_rx) = watch::channel(false);
        self.watchers.write().await.insert(entry.id.clone(), tx);
        self.set_health(&app, &entry.id, ServerHealth::Connecting, None)
            .await;

        let mgr = Arc::clone(self);
        tokio::spawn(async move {
            let mut last_event_id: Option<u64> = None;
            let mut attempt: usize = 0;

            loop {
                if *stop_rx.borrow() {
                    break;
                }
                let client = match Self::build_client(&entry, &tokens) {
                    Ok(c) => c,
                    Err(err) => {
                        mgr.set_health(&app, &entry.id, ServerHealth::Offline, Some(err))
                            .await;
                        break;
                    }
                };

                // Refresh /api/v1/meta on connect so version & capabilities stay fresh.
                match client.meta().await {
                    Ok(meta) => {
                        let mut reg = registry.write().await;
                        let _ = reg.update_meta(&entry.id, meta);
                    }
                    Err(ClientError::Auth(msg)) => {
                        mgr.set_health(&app, &entry.id, ServerHealth::Offline, Some(msg))
                            .await;
                        break;
                    }
                    Err(err) => {
                        mgr.set_health(
                            &app,
                            &entry.id,
                            ServerHealth::Offline,
                            Some(err.to_string()),
                        )
                        .await;
                        let delay = Duration::from_secs(
                            BACKOFF_SECS[attempt.min(BACKOFF_SECS.len() - 1)],
                        );
                        attempt = attempt.saturating_add(1);
                        tokio::select! {
                            _ = stop_rx.wait_for(|&s| s) => break,
                            _ = tokio::time::sleep(delay) => continue,
                        }
                    }
                }

                attempt = 0;
                mgr.set_health(&app, &entry.id, ServerHealth::Online, None)
                    .await;

                let app_clone = app.clone();
                let mgr_clone = Arc::clone(&mgr);
                let sid = entry.id.clone();
                let sname = entry.name.clone();
                let res = client
                    .server_events_stream(last_event_id, stop_rx.clone(), move |ev| {
                        let app_ev = app_clone.clone();
                        let mgr_ev = Arc::clone(&mgr_clone);
                        let sid_ev = sid.clone();
                        let sname_ev = sname.clone();
                        tokio::spawn(async move {
                            mgr_ev
                                .handle_event(&app_ev, &sid_ev, &sname_ev, ev)
                                .await;
                        });
                    })
                    .await;

                if *stop_rx.borrow() {
                    break;
                }

                match res {
                    Ok(latest) => {
                        if latest.is_some() {
                            last_event_id = latest;
                        }
                    }
                    Err(ClientError::Auth(msg)) => {
                        mgr.set_health(&app, &entry.id, ServerHealth::Offline, Some(msg))
                            .await;
                        break;
                    }
                    Err(err) => {
                        mgr.set_health(
                            &app,
                            &entry.id,
                            ServerHealth::Offline,
                            Some(err.to_string()),
                        )
                        .await;
                    }
                }

                let delay =
                    Duration::from_secs(BACKOFF_SECS[attempt.min(BACKOFF_SECS.len() - 1)]);
                attempt = attempt.saturating_add(1);
                tokio::select! {
                    _ = stop_rx.wait_for(|&s| s) => break,
                    _ = tokio::time::sleep(delay) => {}
                }
            }
        });
    }

    pub async fn stop_watcher(&self, server_id: &str) {
        if let Some(tx) = self.watchers.write().await.remove(server_id) {
            let _ = tx.send(true);
        }
    }

    async fn set_health(
        &self,
        app: &AppHandle,
        server_id: &str,
        health: ServerHealth,
        error: Option<String>,
    ) {
        self.health
            .write()
            .await
            .insert(server_id.to_owned(), (health, error.clone()));
        let _ = app.emit(
            "rig://server-status",
            ServerStatusPayload {
                server_id: server_id.to_owned(),
                health,
                error,
            },
        );
    }

    async fn handle_event(
        &self,
        app: &AppHandle,
        server_id: &str,
        server_name: &str,
        ev: ServerEvent,
    ) {
        // Update server health dot from event semantics.
        match ev.kind.as_str() {
            "deploy.started" => {
                self.set_health(app, server_id, ServerHealth::Deploying, None)
                    .await;
            }
            "deploy.finished" => {
                self.set_health(app, server_id, ServerHealth::Online, None)
                    .await;
            }
            "deploy.failed" | "app.crashed" | "app.down" | "disk.low" | "backup.failed" => {
                self.set_health(
                    app,
                    server_id,
                    ServerHealth::Degraded,
                    ev.detail.clone().or_else(|| Some(ev.title.clone())),
                )
                .await;
            }
            _ => {}
        }

        // Record in per-server recent events buffer.
        {
            let mut map = self.recent_events.write().await;
            let buf = map
                .entry(server_id.to_owned())
                .or_insert_with(|| VecDeque::with_capacity(MAX_RECENT_EVENTS));
            if buf.len() >= MAX_RECENT_EVENTS {
                buf.pop_front();
            }
            buf.push_back(ev.clone());
        }

        let _ = app.emit(
            "rig://server-event",
            ServerEventPayload {
                server_id: server_id.to_owned(),
                server_name: server_name.to_owned(),
                event: ev.clone(),
            },
        );

        // Send native OS notification for high-signal events from ANY paired server.
        if matches!(
            ev.kind.as_str(),
            "deploy.finished" | "deploy.failed" | "app.crashed" | "app.down" | "disk.low"
        ) {
            let body = ev.detail.as_deref().unwrap_or(&ev.title);
            let _ = app
                .notification()
                .builder()
                .title(format!("{server_name} · {}", ev.title))
                .body(body)
                .show();
        }
    }
}
