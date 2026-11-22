//! A shell in an app's running container, over a WebSocket.
//!
//! Binary frames carry terminal bytes in both directions. Text frames are
//! JSON control messages from the client: `{"type":"resize","cols":N,"rows":N}`.
//! Being a shell, this is gated at developer and every session is audited.

use std::time::Duration;

use axum::{
    extract::{
        Path, Query, State,
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade, close_code},
    },
    http::{HeaderMap, header},
    response::Response,
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::io::AsyncWriteExt;
use utoipa_axum::router::OpenApiRouter;

use crate::{
    auth::{Auth, Developer, Via, check_origin},
    error::{ApiError, ApiResult},
    runtime::Shell,
    state::AppState,
};

/// A terminal left alone this long is closed, so forgotten tabs do not keep
/// root shells open.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const MAX_FRAME_BYTES: usize = 64 * 1024;
const MAX_COLS: u16 = 500;
const MAX_ROWS: u16 = 200;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().route("/api/v1/apps/{id}/terminal", get(open))
}

#[derive(Deserialize)]
struct Size {
    cols: Option<u16>,
    rows: Option<u16>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Control {
    Resize { cols: u16, rows: u16 },
}

fn clamp(cols: u16, rows: u16) -> (u16, u16) {
    (cols.clamp(20, MAX_COLS), rows.clamp(5, MAX_ROWS))
}

async fn open(
    State(state): State<AppState>,
    auth: Auth<Developer>,
    Path(id): Path<String>,
    Query(size): Query<Size>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> ApiResult<Response> {
    // A WebSocket upgrade is a GET, so the CSRF header check does not apply
    // and the cookie alone would authenticate it. Browsers always send
    // `Origin` on upgrades: require it and require it to be ours.
    if auth.via == Via::Session {
        if !headers.contains_key(header::ORIGIN) {
            return Err(ApiError::Forbidden("cross-origin request blocked"));
        }
        check_origin(&headers)?;
    }

    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT a.name, d.container_id FROM apps a
           LEFT JOIN deployments d ON d.id = a.current_deployment_id
          WHERE a.id = ? AND a.org_id = ? AND a.status = 'running'",
    )
    .bind(&id)
    .bind(&auth.org_id)
    .fetch_optional(&state.db)
    .await?;
    let Some((name, Some(container))) = row else {
        // One answer for "no such app" and "not running" is enough here.
        return Err(ApiError::Conflict(
            "the app is not running, so there is nothing to open a shell in".into(),
        ));
    };

    let (cols, rows) = clamp(size.cols.unwrap_or(80), size.rows.unwrap_or(24));
    let shell = state
        .runtime
        .shell(&container, cols, rows)
        .await
        .map_err(ApiError::Internal)?;
    auth.audit(&state, "app.terminal", Some(&name), None)
        .await?;

    Ok(upgrade
        .max_message_size(MAX_FRAME_BYTES)
        .max_frame_size(MAX_FRAME_BYTES)
        .on_upgrade(move |socket| session(state, socket, shell)))
}

async fn session(state: AppState, socket: WebSocket, shell: Shell) {
    let Shell {
        id,
        mut output,
        mut input,
    } = shell;
    let (mut tx, mut rx) = socket.split();
    let reason = loop {
        tokio::select! {
            chunk = output.next() => match chunk {
                Some(Ok(bytes)) => {
                    if tx.send(Message::Binary(bytes)).await.is_err() {
                        return;
                    }
                }
                // The shell exited (`exit`, ctrl-d) or its container went away.
                Some(Err(_)) | None => break "The shell exited",
            },
            frame = tokio::time::timeout(IDLE_TIMEOUT, rx.next()) => match frame {
                Err(_) => break "Closed after 30 minutes without input",
                Ok(Some(Ok(Message::Binary(bytes)))) => {
                    if input.write_all(&bytes).await.is_err() || input.flush().await.is_err() {
                        break "The shell exited";
                    }
                }
                Ok(Some(Ok(Message::Text(text)))) => {
                    if let Ok(Control::Resize { cols, rows }) = serde_json::from_str(&text) {
                        let (cols, rows) = clamp(cols, rows);
                        state.runtime.resize_shell(&id, cols, rows).await.ok();
                    }
                }
                Ok(Some(Ok(Message::Close(_)))) | Ok(Some(Err(_))) | Ok(None) => return,
                Ok(Some(Ok(_))) => {}
            },
        }
    };
    let close = CloseFrame {
        code: close_code::NORMAL,
        reason: reason.into(),
    };
    tx.send(Message::Close(Some(close))).await.ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_clamped_and_controls_parse() {
        assert_eq!(clamp(0, 0), (20, 5));
        assert_eq!(clamp(9999, 9999), (MAX_COLS, MAX_ROWS));
        assert_eq!(clamp(120, 40), (120, 40));
        let parsed: Control =
            serde_json::from_str(r#"{"type":"resize","cols":100,"rows":30}"#).unwrap();
        assert!(matches!(
            parsed,
            Control::Resize {
                cols: 100,
                rows: 30
            }
        ));
        assert!(serde_json::from_str::<Control>(r#"{"type":"exec","cmd":"rm"}"#).is_err());
    }
}
