//! High-performance Rust log engine for Rig Desktop (§7 performance rules).
//!
//! All heavy log work stays in native Rust:
//! - Bounded ring buffer per stream (default 50,000 lines)
//! - Fast, zero-regex ANSI SGR escape parser into compact styled spans
//! - Substring + stream filtering in Rust
//! - Throttled 60Hz (<= 1 batch per animation frame) delivery to the UI
//! - Built-in synthetic high-rate benchmark stream (5,000+ lines/sec) for
//!   verifying Linux/WebKitGTK smoothness budgets.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

pub const DEFAULT_RING_CAPACITY: usize = 50_000;
/// Maximum batch interval: 16ms (~60fps, at most one batch per animation frame).
pub const FRAME_FLUSH_INTERVAL: Duration = Duration::from_millis(16);
pub const MAX_LINES_PER_BATCH: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnsiSpan {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fg: Option<u8>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub bold: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub dim: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedLogLine {
    pub seq: u64,
    pub ts: String,
    pub stream: String,
    /// Stripped plain text used for fast search & copy.
    pub plain: String,
    /// Compact ANSI spans. Empty when the line had no ANSI styling (UI renders `plain` directly for speed).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spans: Vec<AnsiSpan>,
}

/// Incremental batch pushed over a Tauri `Channel` to the virtualized UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LogChannelFrame {
    Batch {
        stream_id: String,
        lines: Vec<ParsedLogLine>,
        total_buffered: usize,
        dropped_total: u64,
    },
    Status {
        stream_id: String,
        state: String,
        message: Option<String>,
    },
    BenchSummary {
        stream_id: String,
        lines_generated: u64,
        elapsed_ms: u64,
        actual_lines_per_sec: f64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogQueryResult {
    pub total_buffered: usize,
    pub matched_count: usize,
    pub offset: usize,
    pub lines: Vec<ParsedLogLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogBufferStats {
    pub stream_id: String,
    pub buffered_lines: usize,
    pub capacity: usize,
    pub dropped_lines: u64,
}

/// Zero-regex, single-pass ANSI SGR parser.
/// Returns `(plain_text, spans)`. If the line contains no ESC sequences,
/// `spans` is empty so the UI avoids extra DOM nodes per row.
pub fn parse_ansi_line(raw: &str) -> (String, Vec<AnsiSpan>) {
    if !raw.as_bytes().contains(&0x1b) {
        return (raw.to_owned(), Vec::new());
    }

    let bytes = raw.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut plain = String::with_capacity(len);
    let mut spans: Vec<AnsiSpan> = Vec::new();
    let mut cur_fg: Option<u8> = None;
    let mut cur_bold = false;
    let mut cur_dim = false;
    let mut seg_start = 0;

    let flush_seg = |start: usize,
                     end: usize,
                     plain: &mut String,
                     spans: &mut Vec<AnsiSpan>,
                     fg: Option<u8>,
                     bold: bool,
                     dim: bool| {
        if end <= start {
            return;
        }
        let slice = &raw[start..end];
        plain.push_str(slice);
        if let Some(last) = spans.last_mut()
            && last.fg == fg
            && last.bold == bold
            && last.dim == dim
        {
            last.text.push_str(slice);
            return;
        }
        spans.push(AnsiSpan {
            text: slice.to_owned(),
            fg,
            bold,
            dim,
        });
    };

    while i < len {
        if bytes[i] == 0x1b && i + 1 < len && bytes[i + 1] == b'[' {
            flush_seg(seg_start, i, &mut plain, &mut spans, cur_fg, cur_bold, cur_dim);
            i += 2;
            let param_start = i;
            while i < len && !(0x40..=0x7e).contains(&bytes[i]) {
                i += 1;
            }
            if i < len && bytes[i] == b'm' {
                let params = &raw[param_start..i];
                if params.is_empty() {
                    cur_fg = None;
                    cur_bold = false;
                    cur_dim = false;
                } else {
                    for code in params.split(';') {
                        match code.parse::<u16>() {
                            Ok(0) => {
                                cur_fg = None;
                                cur_bold = false;
                                cur_dim = false;
                            }
                            Ok(1) => cur_bold = true,
                            Ok(2) => cur_dim = true,
                            Ok(22) => {
                                cur_bold = false;
                                cur_dim = false;
                            }
                            Ok(30..=37) => cur_fg = Some((code.parse::<u8>().unwrap_or(37)) - 30),
                            Ok(90..=97) => cur_fg = Some((code.parse::<u8>().unwrap_or(97)) - 90 + 8),
                            Ok(39) => cur_fg = None,
                            _ => {}
                        }
                    }
                }
            }
            if i < len {
                i += 1;
            }
            seg_start = i;
        } else {
            i += 1;
        }
    }
    flush_seg(seg_start, len, &mut plain, &mut spans, cur_fg, cur_bold, cur_dim);

    let has_any_style = spans.iter().any(|s| s.fg.is_some() || s.bold || s.dim);
    if !has_any_style {
        spans.clear();
    }
    (plain, spans)
}

pub struct LogRing {
    capacity: usize,
    lines: VecDeque<ParsedLogLine>,
    dropped: u64,
}

impl LogRing {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            lines: VecDeque::with_capacity(capacity.min(4096)),
            dropped: 0,
        }
    }

    pub fn push(&mut self, seq: u64, ts: String, stream: String, raw_line: &str) -> ParsedLogLine {
        let (plain, spans) = parse_ansi_line(raw_line);
        let parsed = ParsedLogLine {
            seq,
            ts,
            stream,
            plain,
            spans,
        };
        if self.lines.len() >= self.capacity {
            self.lines.pop_front();
            self.dropped += 1;
        }
        self.lines.push_back(parsed.clone());
        parsed
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn query(
        &self,
        filter: Option<&str>,
        stream_filter: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> LogQueryResult {
        let needle = filter
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_ascii_lowercase());
        let stream_f = stream_filter
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != "all");

        let mut matched_count = 0usize;
        let mut out = Vec::with_capacity(limit.min(512));

        for line in &self.lines {
            if let Some(sf) = stream_f
                && line.stream != sf
            {
                continue;
            }
            if let Some(q) = &needle
                && !contains_ascii_case_insensitive(&line.plain, q)
            {
                continue;
            }
            if matched_count >= offset && out.len() < limit {
                out.push(line.clone());
            }
            matched_count += 1;
        }

        LogQueryResult {
            total_buffered: self.lines.len(),
            matched_count,
            offset,
            lines: out,
        }
    }
}

fn contains_ascii_case_insensitive(haystack: &str, needle_lower: &str) -> bool {
    let h = haystack.as_bytes();
    let n = needle_lower.as_bytes();
    if n.is_empty() {
        return true;
    }
    if n.len() > h.len() {
        return false;
    }
    h.windows(n.len()).any(|w| {
        w.iter()
            .zip(n.iter())
            .all(|(&a, &b)| a.to_ascii_lowercase() == b)
    })
}

/// Manages active log streams and their ring buffers for the desktop app.
#[derive(Default)]
pub struct LogEngine {
    rings: RwLock<HashMap<String, LogRing>>,
    stops: RwLock<HashMap<String, watch::Sender<bool>>>,
}

impl LogEngine {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Starts (or replaces) a stream with `stream_id`, returning its stop receiver.
    pub fn register_stream(&self, stream_id: &str) -> watch::Receiver<bool> {
        self.stop_stream(stream_id);
        let (tx, rx) = watch::channel(false);
        self.stops
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(stream_id.to_owned(), tx);
        self.rings
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(stream_id.to_owned(), LogRing::new(DEFAULT_RING_CAPACITY));
        rx
    }

    pub fn stop_stream(&self, stream_id: &str) {
        if let Some(tx) = self
            .stops
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .remove(stream_id)
        {
            let _ = tx.send(true);
        }
    }

    pub fn stop_all(&self) {
        let mut stops = self.stops.write().unwrap_or_else(|p| p.into_inner());
        for (_, tx) in stops.drain() {
            let _ = tx.send(true);
        }
    }

    pub fn push_line(
        &self,
        stream_id: &str,
        seq: u64,
        ts: String,
        stream: String,
        raw_line: &str,
    ) -> (ParsedLogLine, usize, u64) {
        let mut rings = self.rings.write().unwrap_or_else(|p| p.into_inner());
        let ring = rings
            .entry(stream_id.to_owned())
            .or_insert_with(|| LogRing::new(DEFAULT_RING_CAPACITY));
        let parsed = ring.push(seq, ts, stream, raw_line);
        (parsed, ring.len(), ring.dropped())
    }

    pub fn query(
        &self,
        stream_id: &str,
        filter: Option<&str>,
        stream_filter: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> LogQueryResult {
        let rings = self.rings.read().unwrap_or_else(|p| p.into_inner());
        match rings.get(stream_id) {
            Some(ring) => ring.query(filter, stream_filter, offset, limit),
            None => LogQueryResult {
                total_buffered: 0,
                matched_count: 0,
                offset,
                lines: Vec::new(),
            },
        }
    }

    pub fn stats(&self) -> Vec<LogBufferStats> {
        let rings = self.rings.read().unwrap_or_else(|p| p.into_inner());
        rings
            .iter()
            .map(|(id, ring)| LogBufferStats {
                stream_id: id.clone(),
                buffered_lines: ring.len(),
                capacity: ring.capacity,
                dropped_lines: ring.dropped(),
            })
            .collect()
    }

    /// Generates a synthetic high-rate ANSI log stream (default 5,000 lines/s)
    /// to validate §7 WebKitGTK rendering & batching budgets.
    pub fn spawn_benchmark(
        self: &Arc<Self>,
        stream_id: String,
        lines_per_sec: u32,
        duration_secs: u32,
        mut emit: impl FnMut(LogChannelFrame) -> bool + Send + 'static,
    ) {
        let engine = Arc::clone(self);
        let mut stop_rx = self.register_stream(&stream_id);
        let target_rate = lines_per_sec.clamp(100, 50_000) as u64;
        let duration = Duration::from_secs(duration_secs.clamp(1, 30) as u64);

        tokio::spawn(async move {
            let started = Instant::now();
            let mut ticker = tokio::time::interval(FRAME_FLUSH_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut seq: u64 = 1;
            let sample_paths = [
                "/api/v1/apps",
                "/api/v1/deployments/0194a8/logs",
                "/healthz",
                "/webhooks/github",
                "/metrics",
            ];

            let _ = emit(LogChannelFrame::Status {
                stream_id: stream_id.clone(),
                state: "streaming".into(),
                message: Some(format!("Benchmark running at {target_rate} lines/s")),
            });

            loop {
                tokio::select! {
                    _ = stop_rx.wait_for(|&s| s) => break,
                    _ = ticker.tick() => {
                        let elapsed = started.elapsed();
                        let desired_total = ((elapsed.as_secs_f64() * target_rate as f64) as u64)
                            .min(target_rate * duration.as_secs());
                        let mut batch = Vec::new();
                        let mut total_buf = 0usize;
                        let mut dropped = 0u64;
                        let now_ts = chrono::Utc::now()
                            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

                        while seq <= desired_total && batch.len() < MAX_LINES_PER_BATCH {
                            let path = sample_paths[(seq as usize) % sample_paths.len()];
                            let (stream, raw) = match seq % 11 {
                                0 => (
                                    "stderr",
                                    format!("\x1b[31;1mERROR\x1b[0m req_id={seq:07} POST {path} status=502 upstream timeout 142ms"),
                                ),
                                3 => (
                                    "deploy",
                                    format!("\x1b[33mWARN\x1b[0m  req_id={seq:07} worker pool saturation 82% active=41/50"),
                                ),
                                _ => (
                                    "stdout",
                                    format!("\x1b[32mINFO\x1b[0m  req_id={seq:07} GET  {path} status=200 latency={}ms", (seq % 17) + 1),
                                ),
                            };
                            let (parsed, buf, drop_n) = engine.push_line(
                                &stream_id,
                                seq,
                                now_ts.clone(),
                                stream.into(),
                                &raw,
                            );
                            batch.push(parsed);
                            total_buf = buf;
                            dropped = drop_n;
                            seq += 1;
                        }

                        if !batch.is_empty()
                            && !emit(LogChannelFrame::Batch {
                                stream_id: stream_id.clone(),
                                lines: batch,
                                total_buffered: total_buf,
                                dropped_total: dropped,
                            })
                        {
                            break;
                        }

                        if elapsed >= duration && seq > target_rate * duration.as_secs() {
                            break;
                        }
                    }
                }
            }

            let elapsed_ms = started.elapsed().as_millis() as u64;
            let generated = seq.saturating_sub(1);
            let actual_rate = if elapsed_ms > 0 {
                (generated as f64) / (elapsed_ms as f64 / 1000.0)
            } else {
                0.0
            };
            let _ = emit(LogChannelFrame::BenchSummary {
                stream_id: stream_id.clone(),
                lines_generated: generated,
                elapsed_ms,
                actual_lines_per_sec: actual_rate,
            });
            let _ = emit(LogChannelFrame::Status {
                stream_id,
                state: "ended".into(),
                message: Some("Benchmark complete".into()),
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi_parser_strips_and_preserves_colors() {
        let (plain, spans) = parse_ansi_line("\x1b[32;1mOK\x1b[0m ready \x1b[31merr\x1b[0m");
        assert_eq!(plain, "OK ready err");
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].fg, Some(2));
        assert!(spans[0].bold);
        assert_eq!(spans[1].fg, None);
        assert_eq!(spans[2].fg, Some(1));

        let (plain2, spans2) = parse_ansi_line("plain text without escape");
        assert_eq!(plain2, "plain text without escape");
        assert!(spans2.is_empty());
    }

    #[test]
    fn ring_buffer_bounds_memory_and_filters_fast() {
        let mut ring = LogRing::new(100);
        for i in 1..=250 {
            let stream = if i % 5 == 0 { "stderr" } else { "stdout" };
            ring.push(
                i,
                "2026-11-24T00:00:00Z".into(),
                stream.into(),
                &format!("line #{i} {}", if i % 10 == 0 { "TIMEOUT" } else { "ok" }),
            );
        }
        assert_eq!(ring.len(), 100);
        assert_eq!(ring.dropped(), 150);

        let q = ring.query(Some("timeout"), Some("stderr"), 0, 50);
        assert_eq!(q.matched_count, 10);
        assert_eq!(q.lines.len(), 10);
        assert_eq!(q.lines[0].seq, 160);
    }
}
