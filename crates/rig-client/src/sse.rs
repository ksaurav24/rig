//! Incremental Server-Sent Events parser.
//!
//! Tracks `event:`, `id:`, and multi-line `data:` fields while ignoring
//! comment lines (`:...`). Operates on raw bytes so chunk boundaries inside
//! multi-byte UTF-8 characters are handled cleanly.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: String,
    pub id: Option<String>,
    pub data: String,
}

#[derive(Default)]
pub struct SseParser {
    buf: Vec<u8>,
    event: String,
    id: Option<String>,
    data: String,
}

impl SseParser {
    pub fn push(&mut self, chunk: &[u8], mut emit: impl FnMut(SseEvent)) {
        self.buf.extend_from_slice(chunk);
        while let Some(end) = self.buf.iter().position(|&b| b == b'\n') {
            let raw: Vec<u8> = self.buf.drain(..=end).collect();
            let line = String::from_utf8_lossy(&raw);
            let line = line.trim_end_matches(['\n', '\r']);
            if line.is_empty() {
                if !self.data.is_empty() || !self.event.is_empty() {
                    let event = if self.event.is_empty() {
                        "message".to_string()
                    } else {
                        std::mem::take(&mut self.event)
                    };
                    let data = std::mem::take(&mut self.data);
                    let id = self.id.take();
                    emit(SseEvent { event, id, data });
                } else {
                    self.event.clear();
                    self.data.clear();
                    self.id = None;
                }
            } else if line.starts_with(':') {
                // Keep-alive / comment line.
            } else if let Some(value) = line.strip_prefix("event:") {
                self.event = value.trim_start().to_string();
            } else if let Some(value) = line.strip_prefix("id:") {
                let value = value.strip_prefix(' ').unwrap_or(value);
                if !value.contains('\0') {
                    self.id = Some(value.to_string());
                }
            } else if let Some(value) = line.strip_prefix("data:") {
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(value.strip_prefix(' ').unwrap_or(value));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_across_chunks_tracks_id_and_utf8() {
        let mut parser = SseParser::default();
        let mut seen = Vec::new();
        let text = ": keep-alive\n\nevent: log\nid: 42\ndata: {\"line\":\"h\u{e9}llo\"}\n\nevent: end\ndata: \"running\"\n\n";
        for b in text.as_bytes() {
            parser.push(&[*b], |ev| seen.push(ev));
        }
        assert_eq!(
            seen,
            vec![
                SseEvent {
                    event: "log".into(),
                    id: Some("42".into()),
                    data: "{\"line\":\"h\u{e9}llo\"}".into(),
                },
                SseEvent {
                    event: "end".into(),
                    id: None,
                    data: "\"running\"".into(),
                },
            ]
        );
    }

    #[test]
    fn multiline_data_and_default_event() {
        let mut parser = SseParser::default();
        let mut seen = Vec::new();
        parser.push(b"id: 7\ndata: line one\ndata: line two\n\n", |ev| {
            seen.push(ev)
        });
        assert_eq!(
            seen,
            vec![SseEvent {
                event: "message".into(),
                id: Some("7".into()),
                data: "line one\nline two".into(),
            }]
        );
    }
}
