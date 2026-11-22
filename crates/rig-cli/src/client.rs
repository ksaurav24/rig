//! HTTP client for the Rig API: JSON calls and server-sent event streams.

use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use reqwest::{Method, Response, StatusCode};
use rig_api::ErrorBody;
use serde::{Serialize, de::DeserializeOwned};

use crate::config::Context;

pub struct Client {
    http: reqwest::Client,
    base: String,
    token: String,
}

impl Client {
    pub fn new(ctx: Context) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("rig-cli/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            // Tokens must not follow a redirect to another host.
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            http,
            base: ctx.url.trim_end_matches('/').to_string(),
            token: ctx.token,
        })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    async fn send<B: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
        stream: bool,
    ) -> Result<Response> {
        let mut req = self
            .http
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(&self.token);
        if !stream {
            req = req.timeout(Duration::from_secs(60));
        }
        if let Some(body) = body {
            req = req.json(body);
        }
        let res = req.send().await.map_err(|e| {
            if e.is_connect() || e.is_timeout() {
                anyhow!(
                    "could not reach {}. Is the server running and the address right?",
                    self.base
                )
            } else {
                anyhow!(e.without_url())
            }
        })?;
        if res.status().is_success() {
            return Ok(res);
        }
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        let message = match serde_json::from_str::<ErrorBody>(&text) {
            Ok(body) => body.error.message,
            Err(_) => format!("the server answered {status}"),
        };
        Err(match status {
            StatusCode::UNAUTHORIZED => {
                anyhow!(
                    "{message}. The token may be expired or deleted. Sign in again with: rig login {}",
                    self.base
                )
            }
            _ => anyhow!(message),
        })
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        decode(self.send(Method::GET, path, None::<&()>, false).await?).await
    }

    pub async fn post<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: Option<&B>,
    ) -> Result<T> {
        decode(self.send(Method::POST, path, body, false).await?).await
    }

    pub async fn put<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> {
        decode(self.send(Method::PUT, path, Some(body), false).await?).await
    }

    pub async fn delete(&self, path: &str) -> Result<()> {
        self.send(Method::DELETE, path, None::<&()>, false)
            .await
            .map(drop)
    }

    /// Reads a server-sent event stream until the server closes it or sends
    /// an `end` event, calling `on_event(event, data)` for each event.
    pub async fn events(&self, path: &str, mut on_event: impl FnMut(&str, &str)) -> Result<()> {
        let mut res = self.send(Method::GET, path, None::<&()>, true).await?;
        let mut parser = SseParser::default();
        while let Some(chunk) = res
            .chunk()
            .await
            .context("the log stream was interrupted")?
        {
            let mut ended = false;
            parser.push(&chunk, |event, data| {
                ended |= event == "end";
                on_event(event, data);
            });
            if ended {
                break;
            }
        }
        Ok(())
    }
}

async fn decode<T: DeserializeOwned>(res: Response) -> Result<T> {
    res.json()
        .await
        .context("the server sent a reply this version of rig does not understand")
}

/// Percent-encodes one path segment.
pub fn segment(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for b in raw.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Incremental parser for the subset of SSE the server emits: `event:`,
/// `data:` (possibly several lines) and comment lines, separated by a blank
/// line. Works on bytes so a chunk boundary inside a UTF-8 character is fine.
#[derive(Default)]
struct SseParser {
    buf: Vec<u8>,
    event: String,
    data: String,
}

impl SseParser {
    fn push(&mut self, chunk: &[u8], mut emit: impl FnMut(&str, &str)) {
        self.buf.extend_from_slice(chunk);
        while let Some(end) = self.buf.iter().position(|&b| b == b'\n') {
            let raw: Vec<u8> = self.buf.drain(..=end).collect();
            let line = String::from_utf8_lossy(&raw);
            let line = line.trim_end_matches(['\n', '\r']);
            if line.is_empty() {
                if !self.data.is_empty() || !self.event.is_empty() {
                    let event = if self.event.is_empty() {
                        "message"
                    } else {
                        self.event.as_str()
                    };
                    emit(event, &self.data);
                }
                self.event.clear();
                self.data.clear();
            } else if let Some(value) = line.strip_prefix("event:") {
                self.event = value.trim_start().to_string();
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
    fn sse_across_chunks() {
        let mut parser = SseParser::default();
        let mut seen = Vec::new();
        let text = ": keep-alive\n\nevent: log\nid: 1\ndata: {\"line\":\"h\u{e9}llo\"}\n\nevent: end\ndata: \"running\"\n\n";
        // Feed one byte at a time, which splits the two-byte character.
        for b in text.as_bytes() {
            parser.push(&[*b], |e, d| seen.push((e.to_string(), d.to_string())));
        }
        assert_eq!(
            seen,
            vec![
                ("log".to_string(), "{\"line\":\"h\u{e9}llo\"}".to_string()),
                ("end".to_string(), "\"running\"".to_string()),
            ]
        );
    }

    #[test]
    fn segments() {
        assert_eq!(segment("API_KEY"), "API_KEY");
        assert_eq!(segment("a/b c"), "a%2Fb%20c");
    }
}
