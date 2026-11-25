//! Native loopback HTTP flow for connecting a GitHub App from Rig Desktop.
//!
//! Spins up an ephemeral `127.0.0.1:<port>` listener, opens `/start` in the
//! user's system browser (which auto-submits the manifest POST form to GitHub),
//! catches GitHub's `/callback?code=...&state=...` redirect, and finishes the
//! manifest conversion against the Rig server using the OS-keychain device token.

use std::time::Duration;

use rig_api::{GithubManifestComplete, GithubManifestRequest, GithubStatus};
use rig_client::Client;
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Mutex, watch},
};

const FLOW_TIMEOUT: Duration = Duration::from_secs(10 * 60);

#[derive(Default)]
pub struct GithubConnectController {
    active_cancel: Mutex<Option<watch::Sender<bool>>>,
}

impl GithubConnectController {
    pub fn new() -> Self {
        Self::default()
    }

    /// Cancels any previous pending GitHub connect listener and returns a fresh cancellation receiver.
    pub async fn start_new(&self) -> watch::Receiver<bool> {
        let (tx, rx) = watch::channel(false);
        let mut guard = self.active_cancel.lock().await;
        if let Some(prev) = guard.replace(tx) {
            let _ = prev.send(true);
        }
        rx
    }

    pub async fn cancel(&self) {
        let mut guard = self.active_cancel.lock().await;
        if let Some(prev) = guard.take() {
            let _ = prev.send(true);
        }
    }
}

pub async fn run_github_connect(
    app: &AppHandle,
    client: &Client,
    organization: Option<String>,
    mut cancel_rx: watch::Receiver<bool>,
) -> Result<GithubStatus, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("failed to bind local callback listener: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("failed to read local callback port: {e}"))?
        .port();

    let redirect_url = format!("http://127.0.0.1:{port}/callback");
    let manifest = client
        .github_manifest(&GithubManifestRequest {
            organization: organization.filter(|s| !s.trim().is_empty()),
            redirect_url: Some(redirect_url),
        })
        .await
        .map_err(|e| e.to_string())?;

    let start_url = format!("http://127.0.0.1:{port}/start");
    app.opener()
        .open_url(&start_url, None::<&str>)
        .map_err(|e| format!("failed to open browser for GitHub authorization: {e}"))?;

    let deadline = tokio::time::Instant::now() + FLOW_TIMEOUT;
    loop {
        let stream = tokio::select! {
            _ = tokio::time::sleep_until(deadline) => {
                return Err("GitHub connection timed out after 10 minutes. Start again.".into());
            }
            _ = cancel_rx.wait_for(|&cancelled| cancelled) => {
                return Err("GitHub connection cancelled.".into());
            }
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => stream,
                    Err(e) => return Err(format!("local callback accept error: {e}")),
                }
            }
        };

        let Some(target) = read_request_target(stream).await else {
            continue;
        };
        let (mut stream, path_and_query) = target;

        if path_and_query == "/start" || path_and_query.starts_with("/start?") {
            let html = render_manifest_form_page(&manifest.post_url, &manifest.manifest);
            let _ = write_html_response(&mut stream, "200 OK", &html).await;
            continue;
        }

        if path_and_query.starts_with("/callback") {
            let (code, state) = parse_callback_params(&path_and_query);
            let (Some(code), Some(state)) = (code, state) else {
                let html = render_result_page(
                    false,
                    "Missing GitHub authorization parameters",
                    "GitHub did not return a code and state. Close this tab and try again from Rig Desktop.",
                    None,
                );
                let _ = write_html_response(&mut stream, "400 Bad Request", &html).await;
                return Err("GitHub callback did not include a code and state.".into());
            };

            match client
                .github_manifest_complete(&GithubManifestComplete { code, state })
                .await
            {
                Ok(status) => {
                    let install_url = status.app.as_ref().map(|a| a.install_url.as_str());
                    let html = render_result_page(
                        true,
                        "GitHub App Connected",
                        "Your GitHub App is connected to this Rig server. Install it on the repositories you want to deploy, then return to Rig Desktop.",
                        install_url,
                    );
                    let _ = write_html_response(&mut stream, "200 OK", &html).await;
                    return Ok(status);
                }
                Err(e) => {
                    let msg = e.to_string();
                    let html = render_result_page(false, "Could not connect GitHub", &msg, None);
                    let _ = write_html_response(&mut stream, "500 Internal Server Error", &html).await;
                    return Err(msg);
                }
            }
        }

        let _ = write_html_response(
            &mut stream,
            "404 Not Found",
            "<!doctype html><title>Not found</title><p>Not found</p>",
        )
        .await;
    }
}

async fn read_request_target(mut stream: TcpStream) -> Option<(TcpStream, String)> {
    let mut buf = [0u8; 8192];
    let n = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf))
        .await
        .ok()?
        .ok()?;
    if n == 0 {
        return None;
    }
    let req = String::from_utf8_lossy(&buf[..n]);
    let first_line = req.lines().next()?;
    let mut parts = first_line.split_whitespace();
    let method = parts.next()?;
    if method != "GET" {
        return None;
    }
    let target = parts.next()?.to_string();
    Some((stream, target))
}

fn parse_callback_params(path_and_query: &str) -> (Option<String>, Option<String>) {
    let Ok(parsed) = url::Url::parse(&format!("http://127.0.0.1{path_and_query}")) else {
        return (None, None);
    };
    let mut code = None;
    let mut state = None;
    for (k, v) in parsed.query_pairs() {
        let val = v.trim();
        if val.is_empty() || val.len() > 256 {
            continue;
        }
        if k == "code" {
            code = Some(val.to_owned());
        } else if k == "state" {
            state = Some(val.to_owned());
        }
    }
    (code, state)
}

async fn write_html_response(
    stream: &mut TcpStream,
    status: &str,
    html: &str,
) -> std::io::Result<()> {
    let resp = format!(
        "HTTP/1.1 {status}\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n\r\n\
         {html}",
        html.len()
    );
    stream.write_all(resp.as_bytes()).await?;
    stream.flush().await
}

fn escape_html(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

fn render_manifest_form_page(post_url: &str, manifest_json: &str) -> String {
    let action = escape_html(post_url);
    let manifest = escape_html(manifest_json);
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width,initial-scale=1" />
<title>Connect GitHub — Rig Desktop</title>
<style>
  body {{ margin:0; min-height:100vh; display:grid; place-items:center; background:#181715; color:#f4f3ef; font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif; }}
  .card {{ max-width:420px; width:90%; background:#22211e; border:1px solid #35332e; border-radius:14px; padding:28px; text-align:center; box-shadow:0 12px 32px rgba(0,0,0,.45); }}
  h1 {{ margin:0 0 8px; font-size:18px; font-weight:600; }}
  p {{ margin:0 0 20px; font-size:14px; color:#a8a59e; line-height:1.5; }}
  button {{ background:#10b981; color:#091410; border:0; border-radius:8px; padding:10px 18px; font-size:14px; font-weight:600; cursor:pointer; }}
  button:hover {{ filter:brightness(1.06); }}
</style>
</head>
<body>
  <div class="card">
    <h1>Redirecting to GitHub…</h1>
    <p>Registering a private GitHub App for your Rig server. Click below if you are not redirected automatically.</p>
    <form id="rig-gh-form" method="post" action="{action}">
      <input type="hidden" name="manifest" value="{manifest}" />
      <button type="submit">Continue to GitHub</button>
    </form>
  </div>
  <script>document.getElementById('rig-gh-form').submit();</script>
</body>
</html>"#
    )
}

fn render_result_page(ok: bool, title: &str, message: &str, install_url: Option<&str>) -> String {
    let accent = if ok { "#10b981" } else { "#f43f5e" };
    let t = escape_html(title);
    let m = escape_html(message);
    let action_html = match install_url {
        Some(url) => format!(
            r#"<a class="btn" href="{}">Choose Repositories on GitHub</a>"#,
            escape_html(url)
        ),
        None => String::new(),
    };
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width,initial-scale=1" />
<title>{t} — Rig Desktop</title>
<style>
  body {{ margin:0; min-height:100vh; display:grid; place-items:center; background:#181715; color:#f4f3ef; font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif; }}
  .card {{ max-width:440px; width:90%; background:#22211e; border:1px solid #35332e; border-radius:14px; padding:28px; text-align:center; box-shadow:0 12px 32px rgba(0,0,0,.45); }}
  .dot {{ width:12px; height:12px; border-radius:999px; background:{accent}; margin:0 auto 14px; }}
  h1 {{ margin:0 0 10px; font-size:19px; font-weight:600; }}
  p {{ margin:0 0 22px; font-size:14px; color:#a8a59e; line-height:1.55; }}
  .btn {{ display:inline-block; background:#10b981; color:#091410; text-decoration:none; border-radius:8px; padding:10px 18px; font-size:14px; font-weight:600; }}
  .btn:hover {{ filter:brightness(1.06); }}
  .hint {{ margin-top:16px; font-size:12.5px; color:#7c7972; }}
</style>
</head>
<body>
  <div class="card">
    <div class="dot"></div>
    <h1>{t}</h1>
    <p>{m}</p>
    {action_html}
    <div class="hint">You can close this tab and return to Rig Desktop.</div>
  </div>
</body>
</html>"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_html_and_parses_callback_query() {
        assert_eq!(escape_html(r#"{"a":"<>&'"}"#), "{&quot;a&quot;:&quot;&lt;&gt;&amp;&#39;&quot;}");
        let (code, state) = parse_callback_params("/callback?code=abc123&state=xyz789");
        assert_eq!(code.as_deref(), Some("abc123"));
        assert_eq!(state.as_deref(), Some("xyz789"));
        assert_eq!(parse_callback_params("/callback"), (None, None));
    }
}
