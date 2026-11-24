//! Lightweight resumable SSE stream of server-wide events for an organization.
//!
//! Drives multi-server health dots, live deploy updates, crash alerts and
//! native notifications in Rig Desktop without polling.

use std::{convert::Infallible, time::Duration};

use axum::{
    extract::{Query, State},
    http::HeaderMap,
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
    routing::get,
};
use rig_api::Role;
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use utoipa_axum::router::OpenApiRouter;

use crate::{
    auth::{self, Auth, Viewer},
    state::AppState,
};

const AUTH_RECHECK_INTERVAL: Duration = Duration::from_secs(30);

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().route("/api/v1/events", get(stream_events))
}

#[derive(Deserialize)]
struct EventsQuery {
    after: Option<u64>,
}

async fn stream_events(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    headers: HeaderMap,
    Query(query): Query<EventsQuery>,
) -> impl IntoResponse {
    let last_event_id = query.after.or_else(|| {
        headers
            .get("last-event-id")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<u64>().ok())
    });

    let snap = auth.snapshot();
    let org_id = auth.org_id.clone();
    let mut rx_bus = state.events.subscribe();
    let backlog = last_event_id
        .map(|after| state.events.since(&org_id, after))
        .unwrap_or_default();

    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(128);
    tokio::spawn(async move {
        let mut last_sent = last_event_id.unwrap_or(0);
        for ev in backlog {
            last_sent = last_sent.max(ev.id);
            let Ok(data) = serde_json::to_string(&ev) else {
                continue;
            };
            let sse = Event::default()
                .event(ev.kind)
                .id(ev.id.to_string())
                .data(data);
            if tx.send(Ok(sse)).await.is_err() {
                return;
            }
        }

        let mut recheck = tokio::time::interval(AUTH_RECHECK_INTERVAL);
        recheck.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        recheck.tick().await;

        loop {
            tokio::select! {
                _ = recheck.tick() => {
                    if !auth::verify_still_valid(&state, &snap, Role::Viewer).await {
                        break;
                    }
                }
                msg = rx_bus.recv() => {
                    match msg {
                        Ok(item) => {
                            if item.org_id != org_id || item.event.id <= last_sent {
                                continue;
                            }
                            last_sent = item.event.id;
                            let Ok(data) = serde_json::to_string(&item.event) else {
                                continue;
                            };
                            let sse = Event::default()
                                .event(item.event.kind)
                                .id(item.event.id.to_string())
                                .data(data);
                            if tx.send(Ok(sse)).await.is_err() {
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            // Catch up from ring buffer if we lagged.
                            for ev in state.events.since(&org_id, last_sent) {
                                last_sent = last_sent.max(ev.id);
                                let Ok(data) = serde_json::to_string(&ev) else {
                                    continue;
                                };
                                let sse = Event::default()
                                    .event(ev.kind)
                                    .id(ev.id.to_string())
                                    .data(data);
                                if tx.send(Ok(sse)).await.is_err() {
                                    return;
                                }
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        }
    });

    Sse::new(ReceiverStream::new(rx))
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(20)))
}
