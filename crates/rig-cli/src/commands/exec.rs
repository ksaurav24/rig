//! `rig exec [app]` — interactive raw-TTY WebSocket shell into a running container.

use std::io::IsTerminal;

use clap::Args;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest, http::HeaderValue};

use crate::api::Client;
use crate::error::{CliError, CliResult};
use crate::ops;

#[derive(Args)]
pub struct ExecArgs {
    /// App name, `project/app`, or id. Defaults to `rig.toml`.
    pub app: Option<String>,
    /// Override initial terminal columns.
    #[arg(long)]
    pub cols: Option<u16>,
    /// Override initial terminal rows.
    #[arg(long)]
    pub rows: Option<u16>,
}

pub async fn run(api: &Client, args: ExecArgs) -> CliResult<()> {
    let app = ops::apps::resolve_app_or_linked(api, args.app.as_deref()).await?;
    let (detected_rows, detected_cols) = terminal_size();
    let cols = args.cols.unwrap_or(detected_cols);
    let rows = args.rows.unwrap_or(detected_rows);

    let ws_url = api.terminal_ws_url(&app.id, cols, rows)?;
    let mut req = ws_url
        .into_client_request()
        .map_err(|e| CliError::message(format!("invalid terminal URL: {e}")))?;
    let auth_val = HeaderValue::from_str(&format!("Bearer {}", api.token()))
        .map_err(|e| CliError::message(format!("invalid token header: {e}")))?;
    req.headers_mut().insert("Authorization", auth_val);

    let (ws, _) = tokio_tungstenite::connect_async(req)
        .await
        .map_err(|e| match e {
            tokio_tungstenite::tungstenite::Error::Http(res) => {
                let code = res.status().as_u16();
                match code {
                    401 | 403 => CliError::auth(format!(
                        "terminal rejected ({code}). Check your token or role (Developer+ required)"
                    )),
                    404 => CliError::not_found("app not found"),
                    409 => CliError::conflict(format!(
                        "{} is not running, so there is nothing to open a shell in",
                        app.name
                    )),
                    _ => CliError::message(format!("terminal upgrade failed with HTTP {code}")),
                }
            }
            other => CliError::network(format!("could not open terminal stream: {other}")),
        })?;

    // Put local terminal into raw mode while attached; Drop guard guarantees restore.
    let _raw_guard = if std::io::stdin().is_terminal() {
        RawModeGuard::enter()?
    } else {
        None
    };

    let (mut ws_tx, mut ws_rx) = ws.split();
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut buf = [0u8; 4096];

    #[cfg(unix)]
    let mut sigwinch =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change()).ok();

    let mut stdin_closed = false;

    loop {
        #[cfg(unix)]
        let resize_fut = async {
            match sigwinch.as_mut() {
                Some(sig) => sig.recv().await,
                None => std::future::pending().await,
            }
        };
        #[cfg(not(unix))]
        let resize_fut = std::future::pending::<Option<()>>();

        tokio::select! {
            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        stdout.write_all(&data).await?;
                        stdout.flush().await?;
                    }
                    Some(Ok(Message::Text(text))) => {
                        stdout.write_all(text.as_bytes()).await?;
                        stdout.flush().await?;
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        let _ = ws_tx.send(Message::Pong(payload)).await;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        return Err(CliError::network(format!("terminal connection closed: {e}")));
                    }
                }
            }
            read = stdin.read(&mut buf), if !stdin_closed => {
                match read {
                    Ok(0) => {
                        stdin_closed = true;
                        let _ = ws_tx.send(Message::Close(None)).await;
                    }
                    Ok(n) => {
                        if ws_tx.send(Message::Binary(buf[..n].to_vec().into())).await.is_err() {
                            break;
                        }
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            Some(_) = resize_fut => {
                let (new_rows, new_cols) = terminal_size();
                let ctrl = json!({
                    "type": "resize",
                    "cols": new_cols,
                    "rows": new_rows,
                })
                .to_string();
                let _ = ws_tx.send(Message::Text(ctrl.into())).await;
            }
        }
    }

    Ok(())
}

fn terminal_size() -> (u16, u16) {
    console::Term::stdout()
        .size_checked()
        .unwrap_or((24, 80))
}

struct RawModeGuard {
    #[cfg(unix)]
    original: libc::termios,
}

impl RawModeGuard {
    fn enter() -> CliResult<Option<Self>> {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let fd = std::io::stdin().as_raw_fd();
            // SAFETY: `tcgetattr` initializes `orig` when returning 0 on a valid tty fd.
            let mut orig = std::mem::MaybeUninit::<libc::termios>::uninit();
            if unsafe { libc::tcgetattr(fd, orig.as_mut_ptr()) } != 0 {
                return Ok(None);
            }
            let original = unsafe { orig.assume_init() };
            let mut raw = original;
            unsafe { libc::cfmakeraw(&mut raw) };
            if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
                return Ok(None);
            }
            Ok(Some(Self { original }))
        }
        #[cfg(not(unix))]
        {
            Ok(None)
        }
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let fd = std::io::stdin().as_raw_fd();
            // SAFETY: Restores the exact termios saved in `enter()`.
            unsafe {
                libc::tcsetattr(fd, libc::TCSANOW, &self.original);
            }
        }
    }
}
