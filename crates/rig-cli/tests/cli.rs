//! End-to-end integration tests driving the compiled `rig` binary against
//! an in-process mock `axum` HTTP/SSE server.

use std::collections::HashMap;
use std::convert::Infallible;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event, Sse},
    },
    routing::{delete, get, post},
};
use rig_api::{
    App, AppStatus, CreateProjectRequest, Database, DatabaseEngine, DatabaseStatus, Deployment,
    DeploymentStatus, EnvVar, ErrorBody, ErrorDetail, LogLine, Me, Organization, Project,
    RevealedEnvVar, Role, SetEnvVarRequest, SourceKind, SystemInfo, User,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio::sync::{Notify, mpsc};
use tokio_stream::wrappers::ReceiverStream;

const VALID_TOKEN: &str = "rig_test_token_12345";
const PROJ_ALPHA_ID: &str = "01940000-0000-7000-8000-0000000000a1";
const PROJ_BETA_ID: &str = "01940000-0000-7000-8000-0000000000b2";
const APP_ALPHA_WEB: &str = "01940000-0000-7000-8000-000000001001";
const APP_BETA_WEB: &str = "01940000-0000-7000-8000-000000001002";
const APP_API: &str = "01940000-0000-7000-8000-000000001003";
const DEP_LIVE: &str = "01940000-0000-7000-8000-0000aaaa1111";
const DEP_PREV: &str = "01940000-0000-7000-8000-0000bbbb1111";
const DEP_OLD: &str = "01940000-0000-7000-8000-0000cccc2222";
const DEP_RESUME: &str = "01940000-0000-7000-8000-0000dddd3333";
const DEP_FAIL: &str = "01940000-0000-7000-8000-0000eeee4444";
const DEP_SLOW: &str = "01940000-0000-7000-8000-0000ffff5555";

#[derive(Default)]
struct MockState {
    /// Recorded `?project_id=` queries on `GET /api/v1/apps`.
    apps_project_queries: Mutex<Vec<Option<String>>>,
    /// Recorded `(after_query, last_event_id_header)` for `DEP_RESUME`.
    resume_requests: Mutex<Vec<(Option<i64>, Option<String>)>>,
    /// Recorded rollback target ids.
    rolled_back: Mutex<Vec<String>>,
    /// Controls which deployment id `POST /apps/{id}/deploy` returns.
    next_deploy_id: Mutex<String>,
    /// Stored env vars per app.
    env_vars: Mutex<HashMap<String, Vec<EnvVar>>>,
    /// Gate to keep `DEP_SLOW` open until released (for MCP concurrency/cancel tests).
    slow_release: Arc<Notify>,
}

fn api_error(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(ErrorBody {
            error: ErrorDetail {
                code: code.into(),
                message: message.into(),
                challenge_token: None,
            },
        }),
    )
        .into_response()
}

fn check_auth(headers: &HeaderMap) -> Option<Response> {
    let ok = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == format!("Bearer {VALID_TOKEN}"));
    if ok {
        None
    } else {
        Some(api_error(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "invalid token",
        ))
    }
}

fn sample_apps() -> Vec<App> {
    vec![
        App {
            id: APP_ALPHA_WEB.into(),
            project_id: PROJ_ALPHA_ID.into(),
            project_name: "Alpha".into(),
            name: "web".into(),
            slug: "web".into(),
            source_kind: SourceKind::Image,
            image: Some("nginx:alpine".into()),
            git_url: None,
            git_branch: None,
            dockerfile_path: "Dockerfile".into(),
            port: 80,
            memory_mb: Some(256),
            cpu_millis: Some(500),
            health_path: Some("/health".into()),
            status: AppStatus::Running,
            current_deployment_id: Some(DEP_LIVE.into()),
            host_port: Some(41001),
            domains: vec!["web.example.com".into()],
            volumes: vec!["/data".into()],
            last_crash: None,
            auto_deploy: false,
            created_at: "2026-11-20T00:00:00Z".into(),
            updated_at: "2026-11-22T00:00:00Z".into(),
        },
        App {
            id: APP_BETA_WEB.into(),
            project_id: PROJ_BETA_ID.into(),
            project_name: "Beta".into(),
            name: "web".into(),
            slug: "web".into(),
            source_kind: SourceKind::Git,
            image: None,
            git_url: Some("https://github.com/acme/web".into()),
            git_branch: Some("main".into()),
            dockerfile_path: "Dockerfile".into(),
            port: 3000,
            memory_mb: None,
            cpu_millis: None,
            health_path: None,
            status: AppStatus::Stopped,
            current_deployment_id: None,
            host_port: None,
            domains: vec![],
            volumes: vec![],
            last_crash: None,
            auto_deploy: true,
            created_at: "2026-11-20T00:00:00Z".into(),
            updated_at: "2026-11-22T00:00:00Z".into(),
        },
        App {
            id: APP_API.into(),
            project_id: PROJ_ALPHA_ID.into(),
            project_name: "Alpha".into(),
            name: "api".into(),
            slug: "api".into(),
            source_kind: SourceKind::Image,
            image: Some("ghcr.io/acme/api:1".into()),
            git_url: None,
            git_branch: None,
            dockerfile_path: "Dockerfile".into(),
            port: 8080,
            memory_mb: None,
            cpu_millis: None,
            health_path: None,
            status: AppStatus::Running,
            current_deployment_id: Some(DEP_LIVE.into()),
            host_port: Some(41002),
            domains: vec!["api.example.com".into()],
            volumes: vec![],
            last_crash: None,
            auto_deploy: false,
            created_at: "2026-11-20T00:00:00Z".into(),
            updated_at: "2026-11-22T00:00:00Z".into(),
        },
    ]
}

fn sample_deployments() -> Vec<Deployment> {
    vec![
        Deployment {
            id: DEP_LIVE.into(),
            app_id: APP_ALPHA_WEB.into(),
            status: DeploymentStatus::Running,
            trigger: "manual".into(),
            image_ref: Some("nginx:1.27".into()),
            commit_sha: Some("1234567890abcdef".into()),
            commit_msg: Some("ship v3".into()),
            error: None,
            created_at: "2026-11-22T12:00:00Z".into(),
            started_at: Some("2026-11-22T12:00:01Z".into()),
            finished_at: Some("2026-11-22T12:00:10Z".into()),
        },
        Deployment {
            id: DEP_PREV.into(),
            app_id: APP_ALPHA_WEB.into(),
            status: DeploymentStatus::Superseded,
            trigger: "api".into(),
            image_ref: Some("nginx:1.26".into()),
            commit_sha: Some("abcdef1234567890".into()),
            commit_msg: Some("ship v2".into()),
            error: None,
            created_at: "2026-11-21T12:00:00Z".into(),
            started_at: Some("2026-11-21T12:00:01Z".into()),
            finished_at: Some("2026-11-21T12:00:10Z".into()),
        },
        Deployment {
            id: DEP_OLD.into(),
            app_id: APP_ALPHA_WEB.into(),
            status: DeploymentStatus::Superseded,
            trigger: "manual".into(),
            image_ref: Some("nginx:1.25".into()),
            commit_sha: None,
            commit_msg: None,
            error: None,
            created_at: "2026-11-20T12:00:00Z".into(),
            started_at: Some("2026-11-20T12:00:01Z".into()),
            finished_at: Some("2026-11-20T12:00:10Z".into()),
        },
    ]
}

fn make_dep(id: &str, app_id: &str, status: DeploymentStatus) -> Deployment {
    Deployment {
        id: id.into(),
        app_id: app_id.into(),
        status,
        trigger: "api".into(),
        image_ref: Some("nginx:alpine".into()),
        commit_sha: None,
        commit_msg: None,
        error: (status == DeploymentStatus::Failed).then(|| "build exited 1".into()),
        created_at: "2026-11-22T12:30:00Z".into(),
        started_at: Some("2026-11-22T12:30:01Z".into()),
        finished_at: status.is_finished().then(|| "2026-11-22T12:30:05Z".into()),
    }
}

fn sse_log(seq: i64, stream: &str, line: &str) -> Event {
    let payload = LogLine {
        seq,
        ts: "2026-11-22T12:30:01Z".into(),
        stream: stream.into(),
        line: line.into(),
    };
    Event::default()
        .event("log")
        .id(seq.to_string())
        .data(serde_json::to_string(&payload).unwrap())
}

#[derive(Deserialize)]
struct AppQuery {
    project_id: Option<String>,
}

#[derive(Deserialize)]
struct LogQuery {
    after: Option<i64>,
}

async fn start_mock_server() -> (String, Arc<MockState>) {
    let state = Arc::new(MockState {
        next_deploy_id: Mutex::new(DEP_RESUME.into()),
        ..Default::default()
    });

    let app = Router::new()
        .route(
            "/api/v1/auth/me",
            get(|headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                Json(Me {
                    user: User {
                        id: "u1".into(),
                        email: "dev@example.com".into(),
                        name: "Dev User".into(),
                        avatar_url: None,
                    },
                    org: Organization {
                        id: "o1".into(),
                        name: "Acme".into(),
                        slug: "acme".into(),
                        logo_url: None,
                        accent_color: "emerald".into(),
                        env_label: None,
                        env_color: None,
                    },
                    role: Role::Owner,
                    csrf_token: "csrf-secret".into(),
                    must_change_password: false,
                })
                .into_response()
            }),
        )
        .route(
            "/api/v1/system",
            get(|headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                Json(SystemInfo {
                    version: "0.1.0".into(),
                    engine: Some("podman".into()),
                    engine_version: Some("5.2.0".into()),
                    proxy_ready: true,
                    cpu_percent: 12.4,
                    memory_used: 512 * 1024 * 1024,
                    memory_total: 2048 * 1024 * 1024,
                    disk_used: 4 * 1024 * 1024 * 1024,
                    disk_total: 20 * 1024 * 1024 * 1024,
                    apps_running: 2,
                    apps_total: 3,
                })
                .into_response()
            }),
        )
        .route(
            "/api/v1/projects",
            get(|headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                Json(vec![
                    Project {
                        id: PROJ_ALPHA_ID.into(),
                        name: "Alpha".into(),
                        slug: "alpha".into(),
                        description: "Primary".into(),
                        app_count: 2,
                        created_at: "2026-11-20T00:00:00Z".into(),
                    },
                    Project {
                        id: PROJ_BETA_ID.into(),
                        name: "Beta".into(),
                        slug: "beta".into(),
                        description: "".into(),
                        app_count: 1,
                        created_at: "2026-11-20T00:00:00Z".into(),
                    },
                ])
                .into_response()
            })
            .post(
                |headers: HeaderMap, Json(req): Json<CreateProjectRequest>| async move {
                    if let Some(r) = check_auth(&headers) {
                        return r;
                    }
                    (
                        StatusCode::CREATED,
                        Json(Project {
                            id: "proj-new".into(),
                            name: req.name.clone(),
                            slug: req.name.to_lowercase(),
                            description: req.description,
                            app_count: 0,
                            created_at: "2026-11-22T12:00:00Z".into(),
                        }),
                    )
                        .into_response()
                },
            ),
        )
        .route(
            "/api/v1/apps",
            get(
                |State(st): State<Arc<MockState>>,
                 headers: HeaderMap,
                 Query(q): Query<AppQuery>| async move {
                    if let Some(r) = check_auth(&headers) {
                        return r;
                    }
                    st.apps_project_queries
                        .lock()
                        .unwrap()
                        .push(q.project_id.clone());
                    let mut apps = sample_apps();
                    if let Some(pid) = q.project_id {
                        apps.retain(|a| a.project_id == pid);
                    }
                    Json(apps).into_response()
                },
            ),
        )
        .route(
            "/api/v1/apps/{id}",
            get(|headers: HeaderMap, Path(id): Path<String>| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                match sample_apps().into_iter().find(|a| a.id == id) {
                    Some(app) => Json(app).into_response(),
                    None => api_error(StatusCode::NOT_FOUND, "not_found", "app not found"),
                }
            }),
        )
        .route(
            "/api/v1/apps/{id}/deploy",
            post(
                |State(st): State<Arc<MockState>>, headers: HeaderMap, Path(id): Path<String>| async move {
                    if let Some(r) = check_auth(&headers) {
                        return r;
                    }
                    let dep_id = st.next_deploy_id.lock().unwrap().clone();
                    (
                        StatusCode::ACCEPTED,
                        Json(make_dep(&dep_id, &id, DeploymentStatus::Queued)),
                    )
                        .into_response()
                },
            ),
        )
        .route(
            "/api/v1/apps/{id}/stop",
            post(|headers: HeaderMap, Path(id): Path<String>| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                let Some(mut app) = sample_apps().into_iter().find(|a| a.id == id) else {
                    return api_error(StatusCode::NOT_FOUND, "not_found", "app not found");
                };
                app.status = AppStatus::Stopped;
                app.current_deployment_id = None;
                Json(app).into_response()
            }),
        )
        .route(
            "/api/v1/apps/{id}/deployments",
            get(|headers: HeaderMap, Path(id): Path<String>| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                let deps: Vec<Deployment> = sample_deployments()
                    .into_iter()
                    .filter(|d| d.app_id == id)
                    .collect();
                Json(deps).into_response()
            }),
        )
        .route(
            "/api/v1/deployments/{id}",
            get(|headers: HeaderMap, Path(id): Path<String>| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                if let Some(d) = sample_deployments().into_iter().find(|d| d.id == id) {
                    return Json(d).into_response();
                }
                match id.as_str() {
                    DEP_RESUME => {
                        Json(make_dep(DEP_RESUME, APP_ALPHA_WEB, DeploymentStatus::Running))
                            .into_response()
                    }
                    DEP_FAIL => Json(make_dep(DEP_FAIL, APP_ALPHA_WEB, DeploymentStatus::Failed))
                        .into_response(),
                    DEP_SLOW => {
                        Json(make_dep(DEP_SLOW, APP_ALPHA_WEB, DeploymentStatus::Building))
                            .into_response()
                    }
                    _ => api_error(StatusCode::NOT_FOUND, "not_found", "deployment not found"),
                }
            }),
        )
        .route(
            "/api/v1/deployments/{id}/rollback",
            post(
                |State(st): State<Arc<MockState>>, headers: HeaderMap, Path(id): Path<String>| async move {
                    if let Some(r) = check_auth(&headers) {
                        return r;
                    }
                    st.rolled_back.lock().unwrap().push(id);
                    let dep_id = st.next_deploy_id.lock().unwrap().clone();
                    (
                        StatusCode::ACCEPTED,
                        Json(make_dep(&dep_id, APP_ALPHA_WEB, DeploymentStatus::Queued)),
                    )
                        .into_response()
                },
            ),
        )
        .route(
            "/api/v1/deployments/{id}/logs",
            get(
                |State(st): State<Arc<MockState>>,
                 headers: HeaderMap,
                 Path(id): Path<String>,
                 Query(q): Query<LogQuery>| async move {
                    if let Some(r) = check_auth(&headers) {
                        return r;
                    }
                    let last_header = headers
                        .get("last-event-id")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_owned);
                    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(16);

                    match id.as_str() {
                        DEP_RESUME => {
                            st.resume_requests
                                .lock()
                                .unwrap()
                                .push((q.after, last_header));
                            let after = q.after.unwrap_or(0);
                            tokio::spawn(async move {
                                if after == 0 {
                                    let _ = tx.send(Ok(sse_log(1, "build", "step 1"))).await;
                                    let _ = tx.send(Ok(sse_log(2, "build", "step 2"))).await;
                                    // Drop connection cleanly without emitting `end` to force resume!
                                } else {
                                    let _ = tx.send(Ok(sse_log(3, "deploy", "step 3"))).await;
                                    let _ = tx
                                        .send(Ok(Event::default().event("end").data("running")))
                                        .await;
                                }
                            });
                        }
                        DEP_FAIL => {
                            tokio::spawn(async move {
                                let _ = tx
                                    .send(Ok(sse_log(1, "stderr", "fatal: compile error")))
                                    .await;
                                let _ = tx
                                    .send(Ok(Event::default().event("end").data("failed")))
                                    .await;
                            });
                        }
                        DEP_SLOW => {
                            let gate = Arc::clone(&st.slow_release);
                            tokio::spawn(async move {
                                let _ = tx.send(Ok(sse_log(1, "build", "building slowly"))).await;
                                gate.notified().await;
                                let _ = tx
                                    .send(Ok(Event::default().event("end").data("running")))
                                    .await;
                            });
                        }
                        DEP_LIVE => {
                            tokio::spawn(async move {
                                let _ = tx.send(Ok(sse_log(1, "build", "live log line"))).await;
                                let _ = tx
                                    .send(Ok(Event::default().event("end").data("running")))
                                    .await;
                            });
                        }
                        _ => {
                            return api_error(
                                StatusCode::NOT_FOUND,
                                "not_found",
                                "deployment not found",
                            );
                        }
                    }

                    Sse::new(ReceiverStream::new(rx)).into_response()
                },
            ),
        )
        .route(
            "/api/v1/apps/{id}/logs",
            get(|headers: HeaderMap, Path(_id): Path<String>| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(8);
                tokio::spawn(async move {
                    let _ = tx.send(Ok(sse_log(1, "stdout", "hello runtime"))).await;
                    let _ = tx.send(Ok(sse_log(2, "stderr", "warn runtime"))).await;
                });
                Sse::new(ReceiverStream::new(rx)).into_response()
            }),
        )
        .route(
            "/api/v1/apps/{id}/env",
            get(
                |State(st): State<Arc<MockState>>, headers: HeaderMap, Path(id): Path<String>| async move {
                    if let Some(r) = check_auth(&headers) {
                        return r;
                    }
                    let vars = st.env_vars.lock().unwrap().get(&id).cloned().unwrap_or_default();
                    Json(vars).into_response()
                },
            )
            .put(
                |State(st): State<Arc<MockState>>,
                 headers: HeaderMap,
                 Path(id): Path<String>,
                 Json(req): Json<SetEnvVarRequest>| async move {
                    if let Some(r) = check_auth(&headers) {
                        return r;
                    }
                    if req.key == "FAIL_ME" {
                        return api_error(
                            StatusCode::UNPROCESSABLE_ENTITY,
                            "validation",
                            "FAIL_ME is rejected",
                        );
                    }
                    let var = EnvVar {
                        key: req.key.clone(),
                        value: (!req.is_secret).then_some(req.value),
                        is_secret: req.is_secret,
                        updated_at: "2026-11-22T12:00:00Z".into(),
                    };
                    let mut map = st.env_vars.lock().unwrap();
                    let list = map.entry(id).or_default();
                    list.retain(|v| v.key != var.key);
                    list.push(var.clone());
                    Json(var).into_response()
                },
            ),
        )
        .route(
            "/api/v1/apps/{id}/env/{key}/reveal",
            post(
                |headers: HeaderMap, Path((_id, key)): Path<(String, String)>| async move {
                    if let Some(r) = check_auth(&headers) {
                        return r;
                    }
                    Json(RevealedEnvVar {
                        key,
                        value: "revealed-secret-val".into(),
                    })
                    .into_response()
                },
            ),
        )
        .route(
            "/api/v1/apps/{id}/env/{key}",
            delete(
                |State(st): State<Arc<MockState>>,
                 headers: HeaderMap,
                 Path((id, key)): Path<(String, String)>| async move {
                    if let Some(r) = check_auth(&headers) {
                        return r;
                    }
                    if let Some(list) = st.env_vars.lock().unwrap().get_mut(&id) {
                        list.retain(|v| v.key != key);
                    }
                    StatusCode::NO_CONTENT.into_response()
                },
            ),
        )
        .route(
            "/api/v1/databases",
            get(|headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                Json(vec![Database {
                    id: "db-1".into(),
                    project_id: PROJ_ALPHA_ID.into(),
                    project_name: "Alpha".into(),
                    name: "pg".into(),
                    engine: DatabaseEngine::Postgres,
                    version: "17".into(),
                    status: DatabaseStatus::Running,
                    error: None,
                    host: "pg".into(),
                    port: 5432,
                    username: "rig".into(),
                    db_name: "pg".into(),
                    host_port: Some(45432),
                    created_at: "2026-11-20T00:00:00Z".into(),
                }])
                .into_response()
            }),
        )
        .with_state(Arc::clone(&state));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (format!("http://{addr}"), state)
}

fn rig_cmd(cfg: &std::path::Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rig"));
    cmd.stdin(Stdio::null())
        .env("RIG_CONFIG", cfg)
        .env_remove("RIG_CONTEXT")
        .env_remove("RIG_URL")
        .env_remove("RIG_TOKEN")
        .env_remove("RIG_INSECURE");
    cmd
}

#[tokio::test]
async fn auth_contexts_stateless_ci_and_exit_codes() {
    let (url, _state) = start_mock_server().await;
    let dir = TempDir::new().unwrap();
    let cfg = dir.path().join("config.toml");

    // 1. Not signed in -> exit code 3 (Auth).
    let out = rig_cmd(&cfg).args(["apps"]).output().await.unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not signed in"));

    // 2. Usage error -> exit code 2.
    let out = rig_cmd(&cfg).args(["not-a-command"]).output().await.unwrap();
    assert_eq!(out.status.code(), Some(2));

    // 3. Unreachable server -> exit code 5 (Network).
    let out = rig_cmd(&cfg)
        .env("RIG_TOKEN", VALID_TOKEN)
        .args(["--server", "http://127.0.0.1:1", "status"])
        .output()
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(5));

    // 4. Stateless CI mode via --server + RIG_TOKEN (no config file exists!).
    let out = rig_cmd(&cfg)
        .env("RIG_TOKEN", VALID_TOKEN)
        .args(["--server", &url, "--json", "status"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let status: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(status["me"]["user"]["email"], "dev@example.com");
    assert_eq!(status["system"]["engine"], "podman");
    assert!(!cfg.exists());

    // 5. Invalid token -> exit code 3 (Auth).
    let out = rig_cmd(&cfg)
        .env("RIG_TOKEN", "wrong")
        .args(["--server", &url, "status"])
        .output()
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(3));

    // 6. Login + context + context use + logout all honor --json.
    let out = rig_cmd(&cfg)
        .env("RIG_TOKEN", VALID_TOKEN)
        .args(["--json", "login", &url, "--name", "prod"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let login_json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(login_json["context"], "prod");
    assert_eq!(login_json["organization"]["name"], "Acme");

    let out = rig_cmd(&cfg)
        .args(["--json", "context"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    let ctxs: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(ctxs.as_array().unwrap().len(), 1);
    assert_eq!(ctxs[0]["name"], "prod");
    assert_eq!(ctxs[0]["current"], true);

    let out = rig_cmd(&cfg)
        .args(["--json", "context", "use", "prod"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["current"],
        "prod"
    );

    let out = rig_cmd(&cfg)
        .args(["context", "use", "missing"])
        .output()
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(4));

    let out = rig_cmd(&cfg)
        .args(["--json", "logout", "prod"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["removed"],
        "prod"
    );
}

#[tokio::test]
async fn apps_projects_server_filter_and_ambiguity() {
    let (url, state) = start_mock_server().await;
    let dir = TempDir::new().unwrap();
    let cfg = dir.path().join("config.toml");

    // `apps --project Alpha` resolves to PROJ_ALPHA_ID and passes ?project_id= to server.
    let out = rig_cmd(&cfg)
        .env("RIG_TOKEN", VALID_TOKEN)
        .args(["--server", &url, "--json", "apps", "--project", "Alpha"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let apps: Vec<App> = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(apps.len(), 2);
    assert!(
        state
            .apps_project_queries
            .lock()
            .unwrap()
            .contains(&Some(PROJ_ALPHA_ID.to_string()))
    );

    // Ambiguous app name `web` fails cleanly listing project/app options.
    let out = rig_cmd(&cfg)
        .env("RIG_TOKEN", VALID_TOKEN)
        .args(["--server", &url, "apps", "show", "web"])
        .output()
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("Alpha/web") && err.contains("Beta/web"), "err={err}");

    // Disambiguated `Alpha/web` succeeds.
    let out = rig_cmd(&cfg)
        .env("RIG_TOKEN", VALID_TOKEN)
        .args(["--server", &url, "--json", "apps", "show", "Alpha/web"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    let app: App = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(app.id, APP_ALPHA_WEB);

    // Unknown app -> exit code 4 (NotFound).
    let out = rig_cmd(&cfg)
        .env("RIG_TOKEN", VALID_TOKEN)
        .args(["--server", &url, "apps", "show", "nonexistent"])
        .output()
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(4));
}

#[tokio::test]
async fn deploy_resumes_dropped_sse_and_failed_deploy_exits_10() {
    let (url, state) = start_mock_server().await;
    let dir = TempDir::new().unwrap();
    let cfg = dir.path().join("config.toml");

    // Deploy whose SSE stream drops after seq 2 and resumes with ?after=2 + Last-Event-ID: 2.
    *state.next_deploy_id.lock().unwrap() = DEP_RESUME.into();
    let out = rig_cmd(&cfg)
        .env("RIG_TOKEN", VALID_TOKEN)
        .args(["--server", &url, "deploy", "Alpha/web"])
        .output()
        .await
        .unwrap();
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("step 1"), "stdout={stdout}");
    assert!(stdout.contains("step 2"), "stdout={stdout}");
    assert!(stdout.contains("step 3"), "stdout={stdout}");
    assert!(stdout.contains("Live at https://web.example.com"), "stdout={stdout}");

    let reqs = state.resume_requests.lock().unwrap().clone();
    assert_eq!(reqs.len(), 2, "expected initial + resumed SSE request: {reqs:?}");
    assert_eq!(reqs[0], (None, None));
    assert_eq!(reqs[1], (Some(2), Some("2".into())));

    // Failed deploy exits with code 10.
    *state.next_deploy_id.lock().unwrap() = DEP_FAIL.into();
    let out = rig_cmd(&cfg)
        .env("RIG_TOKEN", VALID_TOKEN)
        .args(["--server", &url, "deploy", "Alpha/web"])
        .output()
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(10));
    assert!(String::from_utf8_lossy(&out.stderr).contains("deployment failed"));

    // --no-follow --json prints queued deployment JSON immediately.
    let out = rig_cmd(&cfg)
        .env("RIG_TOKEN", VALID_TOKEN)
        .args(["--server", &url, "--json", "deploy", "Alpha/web", "--no-follow"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    let dep: Deployment = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(dep.id, DEP_FAIL);
}

#[tokio::test]
async fn deployments_show_logs_and_strict_rollback() {
    let (url, state) = start_mock_server().await;
    let dir = TempDir::new().unwrap();
    let cfg = dir.path().join("config.toml");

    // Legacy shorthand `rig deployments Alpha/web --json`.
    let out = rig_cmd(&cfg)
        .env("RIG_TOKEN", VALID_TOKEN)
        .args(["--server", &url, "--json", "deployments", "Alpha/web"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    let deps: Vec<Deployment> = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(deps.len(), 3);

    // `rig deployments show <id> --json`.
    let out = rig_cmd(&cfg)
        .env("RIG_TOKEN", VALID_TOKEN)
        .args(["--server", &url, "--json", "deployments", "show", DEP_LIVE])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    let dep: Deployment = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(dep.commit_msg.as_deref(), Some("ship v3"));

    // `rig deployments logs <id> --json` emits NDJSON.
    let out = rig_cmd(&cfg)
        .env("RIG_TOKEN", VALID_TOKEN)
        .args(["--server", &url, "--json", "deployments", "logs", DEP_LIVE])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    let line: LogLine = serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(line.line, "live log line");

    // Rollback rejects < 4 chars.
    let out = rig_cmd(&cfg)
        .env("RIG_TOKEN", VALID_TOKEN)
        .args(["--server", &url, "rollback", "Alpha/web", "1"])
        .output()
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("at least 4"));

    // Rollback rejects ambiguous 4-char suffix (`1111` matches DEP_LIVE and DEP_PREV).
    let out = rig_cmd(&cfg)
        .env("RIG_TOKEN", VALID_TOKEN)
        .args(["--server", &url, "rollback", "Alpha/web", "1111"])
        .output()
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("ambiguous"));

    // Rollback with unique suffix `cccc2222` targets DEP_OLD.
    *state.next_deploy_id.lock().unwrap() = DEP_LIVE.into();
    let out = rig_cmd(&cfg)
        .env("RIG_TOKEN", VALID_TOKEN)
        .args(["--server", &url, "rollback", "Alpha/web", "cccc2222", "--no-follow"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(state.rolled_back.lock().unwrap().last().map(String::as_str), Some(DEP_OLD));
}

#[tokio::test]
async fn env_logs_stop_databases_and_completions_json() {
    let (url, _state) = start_mock_server().await;
    let dir = TempDir::new().unwrap();
    let cfg = dir.path().join("config.toml");

    // env set --json
    let out = rig_cmd(&cfg)
        .env("RIG_TOKEN", VALID_TOKEN)
        .args(["--server", &url, "--json", "env", "set", "api", "FOO=bar", "--plain"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["set"][0]["key"], "FOO");
    assert_eq!(v["set"][0]["value"], "bar");

    // env set partial failure reports which keys were already saved.
    let out = rig_cmd(&cfg)
        .env("RIG_TOKEN", VALID_TOKEN)
        .args(["--server", &url, "env", "set", "api", "GOOD=1", "FAIL_ME=2"])
        .output()
        .await
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("already saved: GOOD"), "err={err}");

    // env get --json
    let out = rig_cmd(&cfg)
        .env("RIG_TOKEN", VALID_TOKEN)
        .args(["--server", &url, "--json", "env", "get", "api", "FOO"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    let rev: RevealedEnvVar = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(rev.value, "revealed-secret-val");

    // env rm --json
    let out = rig_cmd(&cfg)
        .env("RIG_TOKEN", VALID_TOKEN)
        .args(["--server", &url, "--json", "env", "rm", "api", "FOO"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["deleted"],
        "FOO"
    );

    // logs --json emits NDJSON
    let out = rig_cmd(&cfg)
        .env("RIG_TOKEN", VALID_TOKEN)
        .args(["--server", &url, "--json", "logs", "api"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    let lines: Vec<LogLine> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].line, "hello runtime");

    // stop without --yes in non-TTY fails safely, and --yes --json succeeds.
    let out = rig_cmd(&cfg)
        .env("RIG_TOKEN", VALID_TOKEN)
        .args(["--server", &url, "stop", "api"])
        .output()
        .await
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--yes"));

    let out = rig_cmd(&cfg)
        .env("RIG_TOKEN", VALID_TOKEN)
        .args(["--server", &url, "--yes", "--json", "stop", "api"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    let stopped: App = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(stopped.status, AppStatus::Stopped);

    // databases --json
    let out = rig_cmd(&cfg)
        .env("RIG_TOKEN", VALID_TOKEN)
        .args(["--server", &url, "--json", "databases"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    let dbs: Vec<Database> = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(dbs[0].name, "pg");

    // completions bash
    let out = rig_cmd(&cfg)
        .args(["completions", "bash"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("_rig"));
}

#[tokio::test]
async fn mcp_starts_unauthenticated_and_runs_concurrent_with_cancel_and_progress() {
    let (url, state) = start_mock_server().await;
    let dir = TempDir::new().unwrap();
    let cfg = dir.path().join("config.toml");

    // 1. Unauthenticated `rig mcp` must still answer initialize + tools/list
    //    and return isError on tools/call rather than crashing on startup (B4).
    let mut child = rig_cmd(&cfg)
        .args(["mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        stdin
            .write_all(
                concat!(
                    r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#, "\n",
                    r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#, "\n",
                    r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"system_status","arguments":{}}}"#, "\n",
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    }
    let out = child.wait_with_output().await.unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let replies: Vec<Value> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(replies.len(), 3);
    assert_eq!(replies[0]["result"]["serverInfo"]["name"], "rig");
    assert_eq!(replies[1]["result"]["tools"].as_array().unwrap().len(), 32);
    assert_eq!(replies[2]["result"]["isError"], true);
    assert!(
        replies[2]["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("rig login")
    );

    // 2. Concurrent tools/call + progress notifications + notifications/cancelled +
    //    structuredContent + untrusted log wrapper + shared rollback validation (B5/B6).
    *state.next_deploy_id.lock().unwrap() = DEP_SLOW.into();
    let mut child = rig_cmd(&cfg)
        .env("RIG_TOKEN", VALID_TOKEN)
        .args(["--server", &url, "mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap()).lines();

    // Start slow deploy with progressToken.
    stdin
        .write_all(
            format!(
                "{}\n",
                json!({
                    "jsonrpc": "2.0",
                    "id": 10,
                    "method": "tools/call",
                    "params": {
                        "name": "deploy_app",
                        "arguments": { "app": "Alpha/web", "wait": true },
                        "_meta": { "progressToken": "prog-10" }
                    }
                })
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    stdin.flush().await.unwrap();

    // Wait for the progress notification from DEP_SLOW's first log line.
    let first = tokio::time::timeout(Duration::from_secs(5), reader.next_line())
        .await
        .expect("progress notification timed out")
        .unwrap()
        .unwrap();
    let first_val: Value = serde_json::from_str(&first).unwrap();
    assert_eq!(first_val["method"], "notifications/progress");
    assert_eq!(first_val["params"]["progressToken"], "prog-10");

    // While id 10 is still blocked, ping (id 11) must answer immediately!
    stdin
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":11,\"method\":\"ping\"}\n")
        .await
        .unwrap();
    stdin.flush().await.unwrap();
    let ping_line = tokio::time::timeout(Duration::from_secs(2), reader.next_line())
        .await
        .expect("ping blocked by slow deploy")
        .unwrap()
        .unwrap();
    let ping_val: Value = serde_json::from_str(&ping_line).unwrap();
    assert_eq!(ping_val["id"], 11);

    // Cancel id 10, then invoke deployment_logs (id 12) and invalid short rollback (id 13).
    stdin
        .write_all(
            concat!(
                r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":10}}"#, "\n",
                r#"{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"deployment_logs","arguments":{"deployment_id":"01940000-0000-7000-8000-0000aaaa1111"}}}"#, "\n",
                r#"{"jsonrpc":"2.0","id":13,"method":"tools/call","params":{"name":"rollback_app","arguments":{"app":"Alpha/web","deployment_id":"1"}}}"#, "\n",
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    drop(stdin);

    let mut remaining = Vec::new();
    while let Some(line) = tokio::time::timeout(Duration::from_secs(5), reader.next_line())
        .await
        .expect("mcp did not finish after cancel")
        .unwrap()
    {
        remaining.push(serde_json::from_str::<Value>(&line).unwrap());
    }
    let status = child.wait().await.unwrap();
    assert!(status.success());

    // Cancelled id 10 must NOT produce a reply.
    assert!(
        !remaining.iter().any(|v| v.get("id") == Some(&json!(10))),
        "cancelled deploy unexpectedly replied: {remaining:?}"
    );

    let logs_reply = remaining
        .iter()
        .find(|v| v.get("id") == Some(&json!(12)))
        .expect("missing deployment_logs reply");
    assert_eq!(logs_reply["result"]["isError"], false);
    assert_eq!(
        logs_reply["result"]["structuredContent"]["logs"]["untrusted_log_output"],
        true
    );
    assert_eq!(
        logs_reply["result"]["structuredContent"]["logs"]["lines"][0],
        "live log line"
    );

    let rb_reply = remaining
        .iter()
        .find(|v| v.get("id") == Some(&json!(13)))
        .expect("missing rollback_app reply");
    assert_eq!(rb_reply["result"]["isError"], true);
    assert!(
        rb_reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("at least 4")
    );
}
