//! Typed HTTP + SSE client for the Rig API.

pub mod sse;

use std::time::Duration;

use anyhow::Context as _;
use reqwest::{Method, Response, StatusCode, header};
use rig_api::{
    App, CreateProjectRequest, Database, Deployment, EnvVar, ErrorBody, Me, Project,
    RevealedEnvVar, SetEnvVarRequest, SystemInfo,
};
use serde::{Serialize, de::DeserializeOwned};

use crate::config::Context;
use crate::error::{CliError, CliResult};
use sse::{SseEvent, SseParser};

const SSE_BACKOFF_MS: [u64; 5] = [200, 500, 1000, 2000, 4000];

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: String,
    token: String,
}

impl Client {
    pub fn new(ctx: Context) -> CliResult<Self> {
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
        last_event_id: Option<&str>,
    ) -> CliResult<Response> {
        let mut req = self
            .http
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(&self.token);
        if stream {
            // Avoid compressed SSE buffering on proxies and reqwest.
            req = req.header(header::ACCEPT_ENCODING, "identity");
            if let Some(id) = last_event_id {
                req = req.header("Last-Event-ID", id);
            }
        } else {
            req = req.timeout(Duration::from_secs(60));
        }
        if let Some(body) = body {
            req = req.json(body);
        }
        let res = req.send().await.map_err(|e| {
            if e.is_connect() || e.is_timeout() {
                CliError::network(format!(
                    "could not reach {}. Is the server running and the address right?",
                    self.base
                ))
            } else {
                CliError::from(e)
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
            StatusCode::UNAUTHORIZED => CliError::auth(format!(
                "{message}. The token may be expired or deleted. Sign in again with: rig login {}",
                self.base
            )),
            StatusCode::FORBIDDEN => CliError::auth(message),
            StatusCode::NOT_FOUND => CliError::not_found(message),
            _ => CliError::message(message),
        })
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> CliResult<T> {
        decode(self.send(Method::GET, path, None::<&()>, false, None).await?).await
    }

    pub async fn post<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: Option<&B>,
    ) -> CliResult<T> {
        decode(self.send(Method::POST, path, body, false, None).await?).await
    }

    pub async fn put<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> CliResult<T> {
        decode(self.send(Method::PUT, path, Some(body), false, None).await?).await
    }

    pub async fn delete(&self, path: &str) -> CliResult<()> {
        self.send(Method::DELETE, path, None::<&()>, false, None)
            .await
            .map(drop)
    }

    /// Reads an SSE stream once until the server closes it or emits `end`.
    pub async fn events(&self, path: &str, mut on_event: impl FnMut(SseEvent)) -> CliResult<()> {
        let mut res = self
            .send(Method::GET, path, None::<&()>, true, None)
            .await?;
        let mut parser = SseParser::default();
        while let Some(chunk) = res
            .chunk()
            .await
            .map_err(|e| CliError::network(format!("the log stream was interrupted: {}", e.without_url())))?
        {
            let mut ended = false;
            parser.push(&chunk, |ev| {
                ended |= ev.event == "end";
                on_event(ev);
            });
            if ended {
                break;
            }
        }
        Ok(())
    }

    /// Reads a deployment SSE stream, automatically resuming with `Last-Event-ID`
    /// and `?after=<seq>` if the connection drops before an `end` event arrives.
    pub async fn events_resumable(
        &self,
        base_path: &str,
        mut on_event: impl FnMut(SseEvent),
    ) -> CliResult<()> {
        let mut last_id: Option<String> = None;
        let mut attempt: usize = 0;

        loop {
            let path = match &last_id {
                Some(id) => {
                    let sep = if base_path.contains('?') { '&' } else { '?' };
                    format!("{base_path}{sep}after={}", segment(id))
                }
                None => base_path.to_string(),
            };

            let mut res = match self
                .send(Method::GET, &path, None::<&()>, true, last_id.as_deref())
                .await
            {
                Ok(r) => r,
                Err(CliError::Network(msg)) if attempt < SSE_BACKOFF_MS.len() => {
                    let delay = Duration::from_millis(SSE_BACKOFF_MS[attempt]);
                    attempt += 1;
                    tokio::time::sleep(delay).await;
                    let _ = msg;
                    continue;
                }
                Err(e) => return Err(e),
            };

            let mut parser = SseParser::default();
            let mut ended = false;
            let mut stream_err: Option<String> = None;

            loop {
                match res.chunk().await {
                    Ok(Some(chunk)) => {
                        // Receiving data resets the retry counter for subsequent drops.
                        attempt = 0;
                        parser.push(&chunk, |ev| {
                            if let Some(id) = &ev.id {
                                last_id = Some(id.clone());
                            }
                            ended |= ev.event == "end";
                            on_event(ev);
                        });
                        if ended {
                            return Ok(());
                        }
                    }
                    Ok(None) => {
                        // Clean EOF before `end` (e.g. proxy idle timeout): retry if attempts remain.
                        break;
                    }
                    Err(e) => {
                        stream_err = Some(e.without_url().to_string());
                        break;
                    }
                }
            }

            if attempt < SSE_BACKOFF_MS.len() {
                let delay = Duration::from_millis(SSE_BACKOFF_MS[attempt]);
                attempt += 1;
                tokio::time::sleep(delay).await;
                continue;
            }

            return match stream_err {
                Some(e) => Err(CliError::network(format!(
                    "the log stream was interrupted and could not be resumed: {e}"
                ))),
                None => Ok(()),
            };
        }
    }

    // --------------------------------------------------------- typed endpoints

    pub async fn me(&self) -> CliResult<Me> {
        self.get("/api/v1/auth/me").await
    }

    pub async fn system(&self) -> CliResult<SystemInfo> {
        self.get("/api/v1/system").await
    }

    pub async fn list_projects(&self) -> CliResult<Vec<Project>> {
        self.get("/api/v1/projects").await
    }

    pub async fn create_project(&self, req: &CreateProjectRequest) -> CliResult<Project> {
        self.post("/api/v1/projects", Some(req)).await
    }

    pub async fn list_apps(&self, project_id: Option<&str>) -> CliResult<Vec<App>> {
        let path = match project_id {
            Some(id) => format!("/api/v1/apps?project_id={}", segment(id)),
            None => "/api/v1/apps".to_string(),
        };
        self.get(&path).await
    }

    pub async fn get_app(&self, app_id: &str) -> CliResult<App> {
        self.get(&format!("/api/v1/apps/{}", segment(app_id))).await
    }

    pub async fn deploy_app(&self, app_id: &str) -> CliResult<Deployment> {
        self.post(
            &format!("/api/v1/apps/{}/deploy", segment(app_id)),
            None::<&()>,
        )
        .await
    }

    pub async fn stop_app(&self, app_id: &str) -> CliResult<App> {
        self.post(
            &format!("/api/v1/apps/{}/stop", segment(app_id)),
            None::<&()>,
        )
        .await
    }

    pub async fn list_deployments(&self, app_id: &str) -> CliResult<Vec<Deployment>> {
        self.get(&format!("/api/v1/apps/{}/deployments", segment(app_id)))
            .await
    }

    pub async fn get_deployment(&self, deployment_id: &str) -> CliResult<Deployment> {
        self.get(&format!("/api/v1/deployments/{}", segment(deployment_id)))
            .await
    }

    pub async fn rollback_deployment(&self, deployment_id: &str) -> CliResult<Deployment> {
        self.post(
            &format!("/api/v1/deployments/{}/rollback", segment(deployment_id)),
            None::<&()>,
        )
        .await
    }

    pub async fn list_env(&self, app_id: &str) -> CliResult<Vec<EnvVar>> {
        self.get(&format!("/api/v1/apps/{}/env", segment(app_id)))
            .await
    }

    pub async fn set_env(&self, app_id: &str, req: &SetEnvVarRequest) -> CliResult<EnvVar> {
        self.put(&format!("/api/v1/apps/{}/env", segment(app_id)), req)
            .await
    }

    pub async fn reveal_env(&self, app_id: &str, key: &str) -> CliResult<RevealedEnvVar> {
        self.post(
            &format!(
                "/api/v1/apps/{}/env/{}/reveal",
                segment(app_id),
                segment(key)
            ),
            None::<&()>,
        )
        .await
    }

    pub async fn delete_env(&self, app_id: &str, key: &str) -> CliResult<()> {
        self.delete(&format!(
            "/api/v1/apps/{}/env/{}",
            segment(app_id),
            segment(key)
        ))
        .await
    }

    pub async fn list_databases(&self) -> CliResult<Vec<Database>> {
        self.get("/api/v1/databases").await
    }

    pub async fn app_logs_stream(
        &self,
        app_id: &str,
        on_event: impl FnMut(SseEvent),
    ) -> CliResult<()> {
        self.events(&format!("/api/v1/apps/{}/logs", segment(app_id)), on_event)
            .await
    }

    pub async fn deployment_logs_stream(
        &self,
        deployment_id: &str,
        on_event: impl FnMut(SseEvent),
    ) -> CliResult<()> {
        self.events_resumable(
            &format!("/api/v1/deployments/{}/logs", segment(deployment_id)),
            on_event,
        )
        .await
    }
}

async fn decode<T: DeserializeOwned>(res: Response) -> CliResult<T> {
    res.json()
        .await
        .context("the server sent a reply this version of rig does not understand")
        .map_err(CliError::from)
}

/// Percent-encodes one path segment or query value.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments() {
        assert_eq!(segment("API_KEY"), "API_KEY");
        assert_eq!(segment("a/b c"), "a%2Fb%20c");
    }
}
