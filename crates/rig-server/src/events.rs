//! Process-local event bus for the lightweight `GET /api/v1/events` SSE feed.
//!
//! Keeps a small bounded ring buffer of recent events with monotonic sequence
//! IDs so reconnecting desktop/CLI clients can resume cleanly via
//! `Last-Event-ID` or `?after=<id>`.

use std::{
    collections::VecDeque,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use rig_api::ServerEvent;
use tokio::sync::broadcast;

use crate::state::now;

const RING_CAPACITY: usize = 256;
const CHANNEL_CAPACITY: usize = 256;

#[derive(Debug, Clone)]
pub struct OrgEvent {
    pub org_id: String,
    pub event: ServerEvent,
}

pub struct EventBus {
    next_id: AtomicU64,
    tx: broadcast::Sender<OrgEvent>,
    ring: Mutex<VecDeque<OrgEvent>>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self {
            next_id: AtomicU64::new(1),
            tx,
            ring: Mutex::new(VecDeque::with_capacity(RING_CAPACITY)),
        }
    }

    pub fn publish(
        &self,
        org_id: impl Into<String>,
        kind: impl Into<String>,
        title: impl Into<String>,
        detail: Option<String>,
        app_id: Option<String>,
        deployment_id: Option<String>,
        status: Option<String>,
    ) -> ServerEvent {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let event = ServerEvent {
            id,
            kind: kind.into(),
            at: now(),
            title: title.into(),
            detail,
            app_id,
            deployment_id,
            status,
        };
        let wrapped = OrgEvent {
            org_id: org_id.into(),
            event: event.clone(),
        };
        {
            let mut ring = self.ring.lock().unwrap_or_else(|p| p.into_inner());
            if ring.len() >= RING_CAPACITY {
                ring.pop_front();
            }
            ring.push_back(wrapped.clone());
        }
        let _ = self.tx.send(wrapped);
        event
    }

    pub fn subscribe(&self) -> broadcast::Receiver<OrgEvent> {
        self.tx.subscribe()
    }

    /// Returns stored events for `org_id` with `id > after`, in ascending order.
    pub fn since(&self, org_id: &str, after: u64) -> Vec<ServerEvent> {
        let ring = self.ring.lock().unwrap_or_else(|p| p.into_inner());
        ring.iter()
            .filter(|e| e.org_id == org_id && e.event.id > after)
            .map(|e| e.event.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_filters_by_org_and_resumes_after_id() {
        let bus = EventBus::new();
        let e1 = bus.publish("org1", "deploy.started", "Deploying api", None, Some("a1".into()), Some("d1".into()), Some("building".into()));
        let _e2 = bus.publish("org2", "deploy.started", "Deploying other", None, None, None, None);
        let e3 = bus.publish("org1", "deploy.finished", "Deployed api", None, Some("a1".into()), Some("d1".into()), Some("running".into()));

        assert_eq!(e1.id, 1);
        assert_eq!(e3.id, 3);
        let all_org1 = bus.since("org1", 0);
        assert_eq!(all_org1.len(), 2);
        assert_eq!(all_org1[0].id, 1);
        assert_eq!(all_org1[1].id, 3);

        let resumed = bus.since("org1", 1);
        assert_eq!(resumed.len(), 1);
        assert_eq!(resumed[0].id, 3);
    }
}
