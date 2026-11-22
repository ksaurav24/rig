//! Outbound alerts. When something goes wrong that nobody is watching the
//! dashboard for (a failed deploy, a crash, a full disk, a failed backup),
//! Rig POSTs a small JSON event to each channel that asked for it.
//!
//! The body carries both `text` (Slack) and `content` (Discord), so those
//! incoming webhooks work as-is; anything else can read the structured fields.
//! Every request is signed with the channel's own secret in
//! `X-Rig-Signature: sha256=<hex>`, the same scheme push-to-deploy accepts.

use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use hmac::{Hmac, KeyInit, Mac};
use rig_api::AlertEvent;
use serde::Serialize;
use sha2::Sha256;
use sqlx::Row;

use crate::state::{AppState, now};

/// The same alert about the same thing is sent at most once in this window,
/// so a crash loop does not become a message every 15 seconds.
const COOLDOWN: Duration = Duration::from_secs(10 * 60);

static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        // A redirect could send the signed body somewhere the admin did not choose.
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("rig/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("building the alert HTTP client")
});

static RECENT: LazyLock<Mutex<HashMap<String, Instant>>> = LazyLock::new(Default::default);

pub fn event_key(e: AlertEvent) -> &'static str {
    match e {
        AlertEvent::DeployFailed => "deploy.failed",
        AlertEvent::AppCrashed => "app.crashed",
        AlertEvent::AppDown => "app.down",
        AlertEvent::DiskLow => "disk.low",
        AlertEvent::BackupFailed => "backup.failed",
    }
}

pub fn parse_event(s: &str) -> Option<AlertEvent> {
    AlertEvent::ALL.into_iter().find(|e| event_key(*e) == s)
}

/// Only plain http(s) URLs with a host; anything else is refused up front.
pub fn validate_url(raw: &str) -> Result<reqwest::Url> {
    if raw.len() > 2048 {
        bail!("the URL is too long");
    }
    let url = reqwest::Url::parse(raw.trim()).context("that is not a valid URL")?;
    if !matches!(url.scheme(), "http" | "https") {
        bail!("use an http:// or https:// URL");
    }
    if url.host_str().is_none_or(str::is_empty) {
        bail!("the URL needs a host");
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("put credentials in the URL path or query, not before the host");
    }
    Ok(url)
}

/// Free bytes, when they are low enough to warn about: under 5 GiB or 10%,
/// the same line the dashboard banner draws.
pub fn disk_warning(total: u64, used: u64) -> Option<u64> {
    if total == 0 {
        return None;
    }
    let free = total.saturating_sub(used);
    (free < 5 * 1024 * 1024 * 1024 || free < total / 10).then_some(free)
}

#[derive(Debug, Clone)]
pub struct Alert {
    pub event: AlertEvent,
    pub title: String,
    pub detail: String,
    /// What the alert is about, such as an app id. Also scopes the cooldown.
    pub target: Option<String>,
}

#[derive(Serialize)]
struct Payload<'a> {
    event: &'a str,
    title: &'a str,
    detail: &'a str,
    target: Option<&'a str>,
    at: &'a str,
    text: &'a str,
    content: &'a str,
}

fn sign(secret: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC takes any key");
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

/// Whether this alert is outside its cooldown, recording it if so.
fn should_send(org_id: &str, alert: &Alert) -> bool {
    let key = format!(
        "{org_id}|{}|{}",
        event_key(alert.event),
        alert.target.as_deref().unwrap_or("")
    );
    let mut recent = RECENT.lock().unwrap_or_else(|p| p.into_inner());
    let now = Instant::now();
    recent.retain(|_, at| now.duration_since(*at) < COOLDOWN);
    if recent.contains_key(&key) {
        return false;
    }
    recent.insert(key, now);
    true
}

/// Send `alert` to the org's subscribed channels in the background.
/// Delivery problems are recorded on the channel, never raised to the caller.
pub fn notify(state: &AppState, org_id: &str, alert: Alert) {
    if !should_send(org_id, &alert) {
        return;
    }
    let state = state.clone();
    let org_id = org_id.to_owned();
    tokio::spawn(async move {
        if let Err(e) = fan_out(&state, &org_id, &alert).await {
            tracing::warn!("sending {} alert: {e:#}", event_key(alert.event));
        }
    });
}

/// Send `alert` to every organization; for host-wide trouble like disk space.
pub fn notify_all(state: &AppState, alert: Alert) {
    let state = state.clone();
    tokio::spawn(async move {
        let orgs: Vec<String> = match sqlx::query_scalar(
            "SELECT DISTINCT org_id FROM alert_channels WHERE enabled = 1",
        )
        .fetch_all(&state.db)
        .await
        {
            Ok(o) => o,
            Err(e) => return tracing::warn!("listing alert channels: {e}"),
        };
        for org in orgs {
            notify(&state, &org, alert.clone());
        }
    });
}

async fn fan_out(state: &AppState, org_id: &str, alert: &Alert) -> Result<()> {
    let rows = sqlx::query(
        "SELECT id, url_enc, signing_secret, events FROM alert_channels
          WHERE org_id = ? AND enabled = 1",
    )
    .bind(org_id)
    .fetch_all(&state.db)
    .await?;
    let key = event_key(alert.event);
    for r in rows {
        let events: String = r.get("events");
        if !events.split(',').any(|e| e == key) {
            continue;
        }
        let id: String = r.get("id");
        let outcome = deliver(
            state,
            &r.get::<String, _>("url_enc"),
            &r.get::<String, _>("signing_secret"),
            alert,
        )
        .await;
        record(state, &id, outcome.as_ref().err()).await;
    }
    Ok(())
}

/// POST one alert to one channel. Used directly by the "send test" button.
pub async fn deliver(state: &AppState, url_enc: &str, secret: &str, alert: &Alert) -> Result<()> {
    let url = state.vault.decrypt(url_enc)?;
    let at = now();
    let text = format!("{}: {}", alert.title, alert.detail);
    let body = serde_json::to_vec(&Payload {
        event: event_key(alert.event),
        title: &alert.title,
        detail: &alert.detail,
        target: alert.target.as_deref(),
        at: &at,
        text: &text,
        content: &text,
    })?;
    let res = CLIENT
        .post(&url)
        .header("content-type", "application/json")
        .header("x-rig-event", event_key(alert.event))
        .header("x-rig-signature", sign(secret, &body))
        .body(body)
        .send()
        .await
        // The URL may hold a token; keep it out of the message.
        .map_err(|e| anyhow::anyhow!("{}", e.without_url()))?;
    let status = res.status();
    if !status.is_success() {
        bail!("the endpoint answered {status}");
    }
    Ok(())
}

pub async fn record(state: &AppState, channel_id: &str, error: Option<&anyhow::Error>) {
    let res =
        sqlx::query("UPDATE alert_channels SET last_sent_at = ?, last_error = ? WHERE id = ?")
            .bind(now())
            .bind(error.map(|e| format!("{e:#}")))
            .bind(channel_id)
            .execute(&state.db)
            .await;
    if let Err(e) = res {
        tracing::warn!("recording alert delivery: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_checked() {
        assert!(validate_url("https://hooks.slack.com/services/T/B/x").is_ok());
        assert!(validate_url("http://127.0.0.1:9000/hook").is_ok());
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("ftp://example.com").is_err());
        assert!(validate_url("not a url").is_err());
        assert!(validate_url("https://user:pass@example.com").is_err());
        assert!(validate_url(&format!("https://e.com/{}", "a".repeat(3000))).is_err());
    }

    #[test]
    fn disk_warning_matches_the_banner() {
        const G: u64 = 1024 * 1024 * 1024;
        assert_eq!(disk_warning(0, 0), None);
        assert_eq!(disk_warning(100 * G, 50 * G), None);
        assert_eq!(disk_warning(100 * G, 96 * G), Some(4 * G));
        assert_eq!(disk_warning(1000 * G, 950 * G), Some(50 * G));
    }

    #[test]
    fn event_keys_round_trip() {
        for e in AlertEvent::ALL {
            assert_eq!(parse_event(event_key(e)), Some(e));
        }
        assert_eq!(parse_event("nope"), None);
    }

    #[test]
    fn cooldown_is_per_target() {
        let a = |t: &str| Alert {
            event: AlertEvent::AppCrashed,
            title: "t".into(),
            detail: "d".into(),
            target: Some(t.into()),
        };
        assert!(should_send("org-cooldown-test", &a("x")));
        assert!(!should_send("org-cooldown-test", &a("x")));
        assert!(should_send("org-cooldown-test", &a("y")));
    }

    #[test]
    fn signature_matches_the_webhook_scheme() {
        let body = b"{}";
        let sig = sign("k", body);
        let mut h = axum::http::HeaderMap::new();
        h.insert("x-rig-signature", sig.parse().unwrap());
        assert!(crate::routes::hooks::verify("k", &h, body));
    }
}
