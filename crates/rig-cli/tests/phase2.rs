//! Phase 2 end-to-end integration tests driving the compiled `rig` binary
//! against a full-featured mock `axum` HTTP/SSE/WebSocket server.

use std::collections::HashMap;
use std::convert::Infallible;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    extract::{
        Path, Query, State,
        ws::{Message, WebSocketUpgrade},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{
        IntoResponse, Response,
        sse::{Event, Sse},
    },
    routing::{delete, get, patch, post, put},
};
use futures_util::StreamExt;
use rig_api::{
    Access, AddDomainRequest, AddMemberRequest, AlertChannel, AlertEvent, AlertTestResult,
    ApiToken, App, AppStatus, AppWebhook, AuditEntry, Backup, BackupStatus, BulkSetEnvRequest,
    CreateAlertChannelRequest, CreateApiTokenRequest, CreateAppRequest, CreateDatabaseRequest,
    CreateProjectRequest, CreatedAlertChannel, CreatedApiToken, Database, DatabaseCredentials,
    DatabaseEngine, DatabaseEngineInfo, DatabaseStatus, Deployment, DeploymentEndEvent,
    DeploymentStatus, Domain, EnvVar, ErrorBody, ErrorDetail, GithubAppInfo, GithubBranch,
    GithubRepo, GithubStatus, LogLine, Me, Member, MemberWithPassword, MetricPoint, MetricRange,
    MetricSeries, Organization, Project, RevealedEnvVar, Role, SourceKind, SystemInfo, TlsMode,
    UpdateAlertChannelRequest, UpdateAppRequest, UpdateMemberRequest, UpdateProjectRequest, User,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

const TOKEN: &str = "rig_phase2_token_abcdef";
const PROJ_ID: &str = "01940000-0000-7000-8000-0000000000a1";
const PROJ2_ID: &str = "01940000-0000-7000-8000-0000000000b2";
const APP_ID: &str = "01940000-0000-7000-8000-000000001001";
const DB_ID: &str = "01940000-0000-7000-8000-000000002001";
const BK_ID: &str = "01940000-0000-7000-8000-000000003001";
const DEP_ACTIVE: &str = "01940000-0000-7000-8000-0000dddd0001";
const DEP_PREV: &str = "01940000-0000-7000-8000-0000dddd0002";
const DEP_OLD: &str = "01940000-0000-7000-8000-0000dddd0003";

#[derive(Default)]
struct StateData {
    server_version: Mutex<String>,
    conflict_deploy: Mutex<bool>,
    env_vars: Mutex<Vec<EnvVar>>,
    domains: Mutex<Vec<Domain>>,
    tokens: Mutex<Vec<ApiToken>>,
    members: Mutex<Vec<Member>>,
    alerts: Mutex<Vec<AlertChannel>>,
    last_log_query: Mutex<Option<HashMap<String, String>>>,
    last_deploy_query: Mutex<Option<HashMap<String, String>>>,
    terminal_seen_cols_rows: Mutex<Option<(Option<u16>, Option<u16>)>>,
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
        .is_some_and(|v| v == format!("Bearer {TOKEN}"));
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

fn sample_app() -> App {
    App {
        id: APP_ID.into(),
        project_id: PROJ_ID.into(),
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
        current_deployment_id: Some(DEP_ACTIVE.into()),
        host_port: Some(41001),
        domains: vec!["web.example.com".into()],
        volumes: vec![],
        last_crash: None,
        auto_deploy: false,
        created_at: "2026-11-20T00:00:00Z".into(),
        updated_at: "2026-11-22T00:00:00Z".into(),
    }
}

fn sample_db() -> Database {
    Database {
        id: DB_ID.into(),
        project_id: PROJ_ID.into(),
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
    }
}

fn sample_backup() -> Backup {
    Backup {
        id: BK_ID.into(),
        database_id: DB_ID.into(),
        status: BackupStatus::Ok,
        size_bytes: Some(4096),
        error: None,
        restorable: true,
        created_at: "2026-11-22T10:00:00Z".into(),
        finished_at: Some("2026-11-22T10:00:05Z".into()),
    }
}

fn sample_series(range: MetricRange) -> MetricSeries {
    MetricSeries {
        range,
        step_seconds: 60,
        memory_total: 1024 * 1024 * 1024,
        points: vec![
            MetricPoint {
                ts: 1_763_800_000,
                cpu_percent: 12.5,
                memory_bytes: 256 * 1024 * 1024,
                net_rx_bps: Some(1024.0),
                net_tx_bps: Some(2048.0),
            },
            MetricPoint {
                ts: 1_763_800_060,
                cpu_percent: 25.0,
                memory_bytes: 300 * 1024 * 1024,
                net_rx_bps: Some(2048.0),
                net_tx_bps: Some(4096.0),
            },
        ],
    }
}

fn make_dep(id: &str, status: DeploymentStatus, trigger: &str) -> Deployment {
    Deployment {
        id: id.into(),
        app_id: APP_ID.into(),
        status,
        trigger: trigger.into(),
        image_ref: Some("nginx:alpine".into()),
        commit_sha: Some("1234567890abcdef".into()),
        commit_msg: Some("release".into()),
        error: None,
        created_at: "2026-11-22T12:00:00Z".into(),
        started_at: Some("2026-11-22T12:00:01Z".into()),
        finished_at: status.is_finished().then(|| "2026-11-22T12:00:05Z".into()),
    }
}

fn sse_log(seq: i64, stream: &str, line: &str, ts: &str) -> Event {
    let payload = LogLine {
        seq,
        ts: ts.into(),
        stream: stream.into(),
        line: line.into(),
    };
    Event::default()
        .event("log")
        .id(seq.to_string())
        .data(serde_json::to_string(&payload).unwrap())
}

#[derive(Deserialize)]
struct ResolveQ {
    #[serde(rename = "ref")]
    r#ref: String,
}

#[derive(Deserialize)]
struct TermSize {
    cols: Option<u16>,
    rows: Option<u16>,
}

#[derive(Deserialize)]
struct RangeQ {
    #[serde(default)]
    range: MetricRange,
}

async fn start_phase2_server() -> (String, Arc<StateData>) {
    let st = Arc::new(StateData {
        server_version: Mutex::new(env!("CARGO_PKG_VERSION").into()),
        env_vars: Mutex::new(vec![
            EnvVar {
                key: "PLAIN_K".into(),
                value: Some("old-val".into()),
                is_secret: false,
                updated_at: "2026-11-22T00:00:00Z".into(),
            },
            EnvVar {
                key: "SECRET_K".into(),
                value: None,
                is_secret: true,
                updated_at: "2026-11-22T00:00:00Z".into(),
            },
        ]),
        domains: Mutex::new(vec![Domain {
            id: "dom-1".into(),
            host: "web.example.com".into(),
            tls_mode: TlsMode::Auto,
            access: Access::Public,
            port: None,
            created_at: "2026-11-20T00:00:00Z".into(),
        }]),
        tokens: Mutex::new(vec![ApiToken {
            id: "tok-current".into(),
            name: "cli-session".into(),
            prefix: "rig_phase2".into(),
            role: Role::Owner,
            last_used_at: Some("2026-11-22T12:00:00Z".into()),
            expires_at: None,
            created_at: "2026-11-20T00:00:00Z".into(),
        }]),
        members: Mutex::new(vec![
            Member {
                user_id: "u1".into(),
                name: "Owner".into(),
                email: "owner@example.com".into(),
                avatar_url: None,
                role: Role::Owner,
                joined_at: "2026-11-20T00:00:00Z".into(),
                last_login_at: Some("2026-11-22T12:00:00Z".into()),
            },
            Member {
                user_id: "u2".into(),
                name: "Alice".into(),
                email: "alice@example.com".into(),
                avatar_url: None,
                role: Role::Developer,
                joined_at: "2026-11-21T00:00:00Z".into(),
                last_login_at: None,
            },
        ]),
        alerts: Mutex::new(vec![AlertChannel {
            id: "al-1".into(),
            name: "ops".into(),
            url_host: "hooks.slack.com".into(),
            events: vec![AlertEvent::DeployFailed],
            enabled: true,
            last_sent_at: None,
            last_error: None,
            created_at: "2026-11-20T00:00:00Z".into(),
        }]),
        ..Default::default()
    });

    let app = Router::new()
        .route(
            "/api/v1/auth/me",
            get(|State(st): State<Arc<StateData>>, headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                let ver = st.server_version.lock().unwrap().clone();
                let mut res = Json(Me {
                    user: User {
                        id: "u1".into(),
                        email: "owner@example.com".into(),
                        name: "Owner".into(),
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
                    csrf_token: "csrf".into(),
                    must_change_password: false,
                })
                .into_response();
                res.headers_mut()
                    .insert("x-rig-version", HeaderValue::from_str(&ver).unwrap());
                res
            }),
        )
        .route(
            "/api/v1/system",
            get(|State(st): State<Arc<StateData>>, headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                let ver = st.server_version.lock().unwrap().clone();
                Json(SystemInfo {
                    version: ver,
                    engine: Some("podman".into()),
                    engine_version: Some("5.2.0".into()),
                    proxy_ready: true,
                    cpu_percent: 15.0,
                    memory_used: 512 * 1024 * 1024,
                    memory_total: 2048 * 1024 * 1024,
                    disk_used: 2 * 1024 * 1024 * 1024,
                    disk_total: 20 * 1024 * 1024 * 1024,
                    apps_running: 1,
                    apps_total: 1,
                })
                .into_response()
            }),
        )
        .route(
            "/api/v1/system/metrics",
            get(|headers: HeaderMap, Query(q): Query<RangeQ>| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                Json(sample_series(q.range)).into_response()
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
                        id: PROJ_ID.into(),
                        name: "Alpha".into(),
                        slug: "alpha".into(),
                        description: "".into(),
                        app_count: 1,
                        created_at: "2026-11-20T00:00:00Z".into(),
                    },
                    Project {
                        id: PROJ2_ID.into(),
                        name: "Beta".into(),
                        slug: "beta".into(),
                        description: "".into(),
                        app_count: 0,
                        created_at: "2026-11-20T00:00:00Z".into(),
                    },
                ])
                .into_response()
            })
            .post(|headers: HeaderMap, Json(req): Json<CreateProjectRequest>| async move {
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
                        created_at: "2026-11-22T00:00:00Z".into(),
                    }),
                )
                    .into_response()
            }),
        )
        .route(
            "/api/v1/projects/{id}",
            patch(
                |headers: HeaderMap, Path(id): Path<String>, Json(req): Json<UpdateProjectRequest>| async move {
                    if let Some(r) = check_auth(&headers) {
                        return r;
                    }
                    Json(Project {
                        id,
                        name: req.name.unwrap_or_else(|| "Alpha".into()),
                        slug: "alpha".into(),
                        description: req.description.unwrap_or_default(),
                        app_count: 1,
                        created_at: "2026-11-20T00:00:00Z".into(),
                    })
                    .into_response()
                },
            )
            .delete(|headers: HeaderMap, Path(_id): Path<String>| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                StatusCode::NO_CONTENT.into_response()
            }),
        )
        .route(
            "/api/v1/apps/resolve",
            get(|headers: HeaderMap, Query(q): Query<ResolveQ>| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                let r = q.r#ref.trim().to_ascii_lowercase();
                if matches!(r.as_str(), "web" | "alpha/web") || q.r#ref == APP_ID {
                    Json(sample_app()).into_response()
                } else {
                    api_error(StatusCode::NOT_FOUND, "not_found", "app not found")
                }
            }),
        )
        .route(
            "/api/v1/apps",
            get(|headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                Json(vec![sample_app()]).into_response()
            })
            .post(|headers: HeaderMap, Json(req): Json<CreateAppRequest>| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                let mut app = sample_app();
                app.id = "app-created".into();
                app.name = req.name.clone();
                app.slug = req.name.to_lowercase();
                app.source_kind = req.source_kind;
                app.image = req.image;
                app.git_url = req.git_url;
                app.port = req.port;
                app.status = AppStatus::Idle;
                (StatusCode::CREATED, Json(app)).into_response()
            }),
        )
        .route(
            "/api/v1/apps/{id}",
            get(|headers: HeaderMap, Path(id): Path<String>| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                if id == APP_ID {
                    Json(sample_app()).into_response()
                } else {
                    api_error(StatusCode::NOT_FOUND, "not_found", "app not found")
                }
            })
            .patch(
                |headers: HeaderMap, Path(_id): Path<String>, Json(req): Json<UpdateAppRequest>| async move {
                    if let Some(r) = check_auth(&headers) {
                        return r;
                    }
                    let mut app = sample_app();
                    if let Some(p) = req.port {
                        app.port = p;
                    }
                    if let Some(m) = req.memory_mb {
                        app.memory_mb = Some(m);
                    }
                    Json(app).into_response()
                },
            )
            .delete(|headers: HeaderMap, Path(_id): Path<String>| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                StatusCode::NO_CONTENT.into_response()
            }),
        )
        .route(
            "/api/v1/apps/{id}/deploy",
            post(|State(st): State<Arc<StateData>>, headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                if *st.conflict_deploy.lock().unwrap() {
                    return api_error(
                        StatusCode::CONFLICT,
                        "conflict",
                        "a deployment is already in progress",
                    );
                }
                (
                    StatusCode::ACCEPTED,
                    Json(make_dep(DEP_ACTIVE, DeploymentStatus::Queued, "api")),
                )
                    .into_response()
            }),
        )
        .route(
            "/api/v1/apps/{id}/start",
            post(|headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                (
                    StatusCode::ACCEPTED,
                    Json(make_dep(DEP_ACTIVE, DeploymentStatus::Queued, "api")),
                )
                    .into_response()
            }),
        )
        .route(
            "/api/v1/apps/{id}/restart",
            post(|headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                (
                    StatusCode::ACCEPTED,
                    Json(make_dep(DEP_ACTIVE, DeploymentStatus::Queued, "api")),
                )
                    .into_response()
            }),
        )
        .route(
            "/api/v1/apps/{id}/deployments",
            get(
                |State(st): State<Arc<StateData>>,
                 headers: HeaderMap,
                 Query(q): Query<HashMap<String, String>>| async move {
                    if let Some(r) = check_auth(&headers) {
                        return r;
                    }
                    *st.last_deploy_query.lock().unwrap() = Some(q.clone());
                    let mut deps = vec![
                        make_dep(
                            DEP_ACTIVE,
                            if *st.conflict_deploy.lock().unwrap() {
                                DeploymentStatus::Building
                            } else {
                                DeploymentStatus::Running
                            },
                            "api",
                        ),
                        make_dep(DEP_PREV, DeploymentStatus::Superseded, "manual"),
                        make_dep(DEP_OLD, DeploymentStatus::Superseded, "manual"),
                    ];
                    if let Some(lim) = q.get("limit").and_then(|s| s.parse::<usize>().ok()) {
                        deps.truncate(lim);
                    }
                    Json(deps).into_response()
                },
            ),
        )
        .route(
            "/api/v1/deployments/{id}",
            get(|headers: HeaderMap, Path(id): Path<String>| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                Json(make_dep(&id, DeploymentStatus::Running, "api")).into_response()
            }),
        )
        .route(
            "/api/v1/deployments/{id}/rollback",
            post(|headers: HeaderMap, Path(_id): Path<String>| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                (
                    StatusCode::ACCEPTED,
                    Json(make_dep(DEP_ACTIVE, DeploymentStatus::Queued, "rollback")),
                )
                    .into_response()
            }),
        )
        .route(
            "/api/v1/deployments/{id}/cancel",
            post(|headers: HeaderMap, Path(id): Path<String>| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                Json(make_dep(&id, DeploymentStatus::Cancelled, "api")).into_response()
            }),
        )
        .route(
            "/api/v1/deployments/{id}/logs",
            get(|headers: HeaderMap, Path(_id): Path<String>| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(8);
                tokio::spawn(async move {
                    let _ = tx
                        .send(Ok(sse_log(1, "build", "building image", "2026-11-22T12:00:01Z")))
                        .await;
                    let end = serde_json::to_string(&DeploymentEndEvent {
                        status: "running".into(),
                        domains: vec!["web.example.com".into()],
                    })
                    .unwrap();
                    let _ = tx.send(Ok(Event::default().event("end").data(end))).await;
                });
                Sse::new(ReceiverStream::new(rx)).into_response()
            }),
        )
        .route(
            "/api/v1/apps/{id}/logs",
            get(
                |State(st): State<Arc<StateData>>,
                 headers: HeaderMap,
                 Query(q): Query<HashMap<String, String>>| async move {
                    if let Some(r) = check_auth(&headers) {
                        return r;
                    }
                    *st.last_log_query.lock().unwrap() = Some(q.clone());
                    let grep = q.get("grep").map(|g| g.to_lowercase());
                    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(8);
                    tokio::spawn(async move {
                        for (seq, stream, line, ts) in [
                            (1, "stdout", "info: started", "2026-11-22T12:00:01Z"),
                            (2, "stderr", "WARN: slow query", "2026-11-22T12:00:02Z"),
                        ] {
                            if grep
                                .as_deref()
                                .is_none_or(|g| line.to_lowercase().contains(g))
                            {
                                let _ = tx.send(Ok(sse_log(seq, stream, line, ts))).await;
                            }
                        }
                    });
                    Sse::new(ReceiverStream::new(rx)).into_response()
                },
            ),
        )
        .route(
            "/api/v1/apps/{id}/terminal",
            get(
                |State(st): State<Arc<StateData>>,
                 headers: HeaderMap,
                 Query(sz): Query<TermSize>,
                 ws: WebSocketUpgrade| async move {
                    if let Some(r) = check_auth(&headers) {
                        return r;
                    }
                    *st.terminal_seen_cols_rows.lock().unwrap() = Some((sz.cols, sz.rows));
                    ws.on_upgrade(|mut socket| async move {
                        let _ = socket.send(Message::Binary(b"welcome> ".to_vec().into())).await;
                        if let Some(Ok(Message::Binary(data))) = socket.next().await {
                            let mut echoed = b"echo:".to_vec();
                            echoed.extend_from_slice(&data);
                            let _ = socket.send(Message::Binary(echoed.into())).await;
                        }
                        let _ = socket.send(Message::Close(None)).await;
                    })
                    .into_response()
                },
            ),
        )
        .route(
            "/api/v1/apps/{id}/env",
            get(|State(st): State<Arc<StateData>>, headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                Json(st.env_vars.lock().unwrap().clone()).into_response()
            })
            .put(
                |State(st): State<Arc<StateData>>,
                 headers: HeaderMap,
                 Json(req): Json<rig_api::SetEnvVarRequest>| async move {
                    if let Some(r) = check_auth(&headers) {
                        return r;
                    }
                    let var = EnvVar {
                        key: req.key.clone(),
                        value: (!req.is_secret).then_some(req.value),
                        is_secret: req.is_secret,
                        updated_at: "2026-11-22T12:00:00Z".into(),
                    };
                    let mut list = st.env_vars.lock().unwrap();
                    list.retain(|v| v.key != var.key);
                    list.push(var.clone());
                    Json(var).into_response()
                },
            ),
        )
        .route(
            "/api/v1/apps/{id}/env/bulk",
            put(
                |State(st): State<Arc<StateData>>,
                 headers: HeaderMap,
                 Json(req): Json<BulkSetEnvRequest>| async move {
                    if let Some(r) = check_auth(&headers) {
                        return r;
                    }
                    let mut list = st.env_vars.lock().unwrap();
                    if req.replace {
                        list.clear();
                    }
                    let mut saved = Vec::new();
                    for item in req.vars {
                        let var = EnvVar {
                            key: item.key.clone(),
                            value: (!item.is_secret).then_some(item.value),
                            is_secret: item.is_secret,
                            updated_at: "2026-11-22T12:00:00Z".into(),
                        };
                        list.retain(|v| v.key != var.key);
                        list.push(var.clone());
                        saved.push(var);
                    }
                    Json(saved).into_response()
                },
            ),
        )
        .route(
            "/api/v1/apps/{id}/env/{key}/reveal",
            post(|headers: HeaderMap, Path((_id, key)): Path<(String, String)>| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                Json(RevealedEnvVar {
                    key,
                    value: "s3cr3t".into(),
                })
                .into_response()
            }),
        )
        .route(
            "/api/v1/apps/{id}/domains",
            get(|State(st): State<Arc<StateData>>, headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                Json(st.domains.lock().unwrap().clone()).into_response()
            })
            .post(
                |State(st): State<Arc<StateData>>,
                 headers: HeaderMap,
                 Json(req): Json<AddDomainRequest>| async move {
                    if let Some(r) = check_auth(&headers) {
                        return r;
                    }
                    let d = Domain {
                        id: format!("dom-{}", req.host),
                        host: req.host,
                        tls_mode: req.tls_mode,
                        access: req.access,
                        port: req.port,
                        created_at: "2026-11-22T12:00:00Z".into(),
                    };
                    st.domains.lock().unwrap().push(d.clone());
                    (StatusCode::CREATED, Json(d)).into_response()
                },
            ),
        )
        .route(
            "/api/v1/apps/{id}/domains/{dom_id}",
            delete(
                |State(st): State<Arc<StateData>>,
                 headers: HeaderMap,
                 Path((_id, dom_id)): Path<(String, String)>| async move {
                    if let Some(r) = check_auth(&headers) {
                        return r;
                    }
                    st.domains.lock().unwrap().retain(|d| d.id != dom_id);
                    StatusCode::NO_CONTENT.into_response()
                },
            ),
        )
        .route(
            "/api/v1/apps/{id}/metrics",
            get(|headers: HeaderMap, Query(q): Query<RangeQ>| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                Json(sample_series(q.range)).into_response()
            }),
        )
        .route(
            "/api/v1/apps/{id}/webhook",
            get(|headers: HeaderMap, Path(id): Path<String>| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                Json(AppWebhook {
                    path: format!("/api/v1/hooks/apps/{id}"),
                    secret: "wh_sec_1".into(),
                })
                .into_response()
            }),
        )
        .route(
            "/api/v1/apps/{id}/webhook/rotate",
            post(|headers: HeaderMap, Path(id): Path<String>| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                Json(AppWebhook {
                    path: format!("/api/v1/hooks/apps/{id}"),
                    secret: "wh_sec_2".into(),
                })
                .into_response()
            }),
        )
        .route(
            "/api/v1/databases",
            get(|headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                Json(vec![sample_db()]).into_response()
            })
            .post(|headers: HeaderMap, Json(req): Json<CreateDatabaseRequest>| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                let mut db = sample_db();
                db.name = req.name;
                db.engine = req.engine;
                if let Some(v) = req.version {
                    db.version = v;
                }
                (StatusCode::ACCEPTED, Json(db)).into_response()
            }),
        )
        .route(
            "/api/v1/databases/engines",
            get(|headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                Json(vec![DatabaseEngineInfo {
                    engine: DatabaseEngine::Postgres,
                    label: "PostgreSQL".into(),
                    versions: vec!["17".into(), "16".into()],
                    port: 5432,
                }])
                .into_response()
            }),
        )
        .route(
            "/api/v1/databases/{id}",
            get(|headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                Json(sample_db()).into_response()
            })
            .delete(|headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                StatusCode::NO_CONTENT.into_response()
            }),
        )
        .route(
            "/api/v1/databases/{id}/start",
            post(|headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                (StatusCode::ACCEPTED, Json(sample_db())).into_response()
            }),
        )
        .route(
            "/api/v1/databases/{id}/stop",
            post(|headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                let mut db = sample_db();
                db.status = DatabaseStatus::Stopped;
                Json(db).into_response()
            }),
        )
        .route(
            "/api/v1/databases/{id}/reveal",
            post(|headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                Json(DatabaseCredentials {
                    password: "dbpass".into(),
                    url: "postgres://rig:dbpass@pg:5432/pg".into(),
                })
                .into_response()
            }),
        )
        .route(
            "/api/v1/databases/{id}/metrics",
            get(|headers: HeaderMap, Query(q): Query<RangeQ>| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                Json(sample_series(q.range)).into_response()
            }),
        )
        .route(
            "/api/v1/databases/{id}/backups",
            get(|headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                Json(vec![sample_backup()]).into_response()
            })
            .post(|headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                (StatusCode::ACCEPTED, Json(sample_backup())).into_response()
            }),
        )
        .route(
            "/api/v1/backups/{id}",
            delete(|headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                StatusCode::NO_CONTENT.into_response()
            }),
        )
        .route(
            "/api/v1/backups/{id}/restore",
            post(|headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                StatusCode::NO_CONTENT.into_response()
            }),
        )
        .route(
            "/api/v1/backups/{id}/download",
            get(|headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                let mut res = b"-- pg dump payload\n".to_vec().into_response();
                res.headers_mut().insert(
                    header::CONTENT_DISPOSITION,
                    HeaderValue::from_static("attachment; filename=\"pg-2026.sql\""),
                );
                res
            }),
        )
        .route(
            "/api/v1/tokens",
            get(|State(st): State<Arc<StateData>>, headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                Json(st.tokens.lock().unwrap().clone()).into_response()
            })
            .post(
                |State(st): State<Arc<StateData>>,
                 headers: HeaderMap,
                 Json(req): Json<CreateApiTokenRequest>| async move {
                    if let Some(r) = check_auth(&headers) {
                        return r;
                    }
                    let tok = ApiToken {
                        id: "tok-new".into(),
                        name: req.name,
                        prefix: "rig_newtok".into(),
                        role: req.role,
                        last_used_at: None,
                        expires_at: None,
                        created_at: "2026-11-22T12:00:00Z".into(),
                    };
                    st.tokens.lock().unwrap().push(tok.clone());
                    Json(CreatedApiToken {
                        token: tok,
                        secret: "rig_newtok_secret_value".into(),
                    })
                    .into_response()
                },
            ),
        )
        .route(
            "/api/v1/tokens/{id}",
            delete(
                |State(st): State<Arc<StateData>>,
                 headers: HeaderMap,
                 Path(id): Path<String>| async move {
                    if let Some(r) = check_auth(&headers) {
                        return r;
                    }
                    st.tokens.lock().unwrap().retain(|t| t.id != id);
                    StatusCode::NO_CONTENT.into_response()
                },
            ),
        )
        .route(
            "/api/v1/members",
            get(|State(st): State<Arc<StateData>>, headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                Json(st.members.lock().unwrap().clone()).into_response()
            })
            .post(
                |State(st): State<Arc<StateData>>,
                 headers: HeaderMap,
                 Json(req): Json<AddMemberRequest>| async move {
                    if let Some(r) = check_auth(&headers) {
                        return r;
                    }
                    let m = Member {
                        user_id: "u3".into(),
                        name: req.name,
                        email: req.email,
                        avatar_url: None,
                        role: req.role,
                        joined_at: "2026-11-22T12:00:00Z".into(),
                        last_login_at: None,
                    };
                    st.members.lock().unwrap().push(m.clone());
                    Json(MemberWithPassword {
                        member: m,
                        temporary_password: "temp-pw-123".into(),
                    })
                    .into_response()
                },
            ),
        )
        .route(
            "/api/v1/members/{uid}",
            patch(
                |State(st): State<Arc<StateData>>,
                 headers: HeaderMap,
                 Path(uid): Path<String>,
                 Json(req): Json<UpdateMemberRequest>| async move {
                    if let Some(r) = check_auth(&headers) {
                        return r;
                    }
                    let mut list = st.members.lock().unwrap();
                    let m = list.iter_mut().find(|m| m.user_id == uid).unwrap();
                    m.role = req.role;
                    Json(m.clone()).into_response()
                },
            )
            .delete(
                |State(st): State<Arc<StateData>>,
                 headers: HeaderMap,
                 Path(uid): Path<String>| async move {
                    if let Some(r) = check_auth(&headers) {
                        return r;
                    }
                    st.members.lock().unwrap().retain(|m| m.user_id != uid);
                    StatusCode::NO_CONTENT.into_response()
                },
            ),
        )
        .route(
            "/api/v1/members/{uid}/reset-password",
            post(
                |State(st): State<Arc<StateData>>,
                 headers: HeaderMap,
                 Path(uid): Path<String>| async move {
                    if let Some(r) = check_auth(&headers) {
                        return r;
                    }
                    let m = st
                        .members
                        .lock()
                        .unwrap()
                        .iter()
                        .find(|m| m.user_id == uid)
                        .unwrap()
                        .clone();
                    Json(MemberWithPassword {
                        member: m,
                        temporary_password: "reset-pw-456".into(),
                    })
                    .into_response()
                },
            ),
        )
        .route(
            "/api/v1/audit",
            get(|headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                Json(vec![
                    AuditEntry {
                        id: "aud-1".into(),
                        actor: "owner@example.com".into(),
                        action: "app.deploy".into(),
                        target: Some(APP_ID.into()),
                        detail: Some("v1".into()),
                        ip: Some("127.0.0.1".into()),
                        created_at: "2026-11-22T12:00:00Z".into(),
                    },
                    AuditEntry {
                        id: "aud-2".into(),
                        actor: "alice@example.com".into(),
                        action: "env.set".into(),
                        target: Some(APP_ID.into()),
                        detail: Some("PORT".into()),
                        ip: None,
                        created_at: "2026-11-22T11:00:00Z".into(),
                    },
                ])
                .into_response()
            }),
        )
        .route(
            "/api/v1/alerts/channels",
            get(|State(st): State<Arc<StateData>>, headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                Json(st.alerts.lock().unwrap().clone()).into_response()
            })
            .post(
                |State(st): State<Arc<StateData>>,
                 headers: HeaderMap,
                 Json(req): Json<CreateAlertChannelRequest>| async move {
                    if let Some(r) = check_auth(&headers) {
                        return r;
                    }
                    let ch = AlertChannel {
                        id: "al-2".into(),
                        name: req.name,
                        url_host: "example.com".into(),
                        events: req.events,
                        enabled: true,
                        last_sent_at: None,
                        last_error: None,
                        created_at: "2026-11-22T12:00:00Z".into(),
                    };
                    st.alerts.lock().unwrap().push(ch.clone());
                    (
                        StatusCode::CREATED,
                        Json(CreatedAlertChannel {
                            channel: ch,
                            signing_secret: "al_sig_secret".into(),
                        }),
                    )
                        .into_response()
                },
            ),
        )
        .route(
            "/api/v1/alerts/channels/{id}",
            patch(
                |State(st): State<Arc<StateData>>,
                 headers: HeaderMap,
                 Path(id): Path<String>,
                 Json(req): Json<UpdateAlertChannelRequest>| async move {
                    if let Some(r) = check_auth(&headers) {
                        return r;
                    }
                    let mut list = st.alerts.lock().unwrap();
                    let ch = list.iter_mut().find(|c| c.id == id).unwrap();
                    if let Some(e) = req.enabled {
                        ch.enabled = e;
                    }
                    Json(ch.clone()).into_response()
                },
            )
            .delete(
                |State(st): State<Arc<StateData>>,
                 headers: HeaderMap,
                 Path(id): Path<String>| async move {
                    if let Some(r) = check_auth(&headers) {
                        return r;
                    }
                    st.alerts.lock().unwrap().retain(|c| c.id != id);
                    StatusCode::NO_CONTENT.into_response()
                },
            ),
        )
        .route(
            "/api/v1/alerts/channels/{id}/test",
            post(|headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                Json(AlertTestResult {
                    ok: true,
                    error: None,
                })
                .into_response()
            }),
        )
        .route(
            "/api/v1/github",
            get(|headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                Json(GithubStatus {
                    app: Some(GithubAppInfo {
                        name: "Rig Bot".into(),
                        slug: "rig-bot".into(),
                        html_url: "https://github.com/apps/rig-bot".into(),
                        owner: "acme".into(),
                        install_url: "https://github.com/apps/rig-bot/installations/new".into(),
                        created_at: "2026-11-20T00:00:00Z".into(),
                    }),
                    poll_interval: 60,
                    webhook_url: "/api/v1/hooks/github/o1".into(),
                })
                .into_response()
            }),
        )
        .route(
            "/api/v1/github/repos",
            get(|headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                Json(vec![GithubRepo {
                    full_name: "acme/web".into(),
                    clone_url: "https://github.com/acme/web.git".into(),
                    default_branch: "main".into(),
                    private: true,
                }])
                .into_response()
            }),
        )
        .route(
            "/api/v1/github/repos/{owner}/{repo}/branches",
            get(|headers: HeaderMap| async move {
                if let Some(r) = check_auth(&headers) {
                    return r;
                }
                Json(vec![GithubBranch {
                    name: "main".into(),
                    default: true,
                    protected: true,
                }])
                .into_response()
            }),
        )
        .with_state(Arc::clone(&st));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), st)
}

fn rig_cmd(cfg: &std::path::Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rig"));
    cmd.stdin(Stdio::null())
        .env("RIG_CONFIG", cfg)
        .env("RIG_TOKEN", TOKEN)
        .env_remove("RIG_CONTEXT")
        .env_remove("RIG_URL")
        .env_remove("RIG_INSECURE");
    cmd
}

#[tokio::test]
async fn exec_websocket_terminal_streams_binary_and_passes_size() {
    let (url, st) = start_phase2_server().await;
    let dir = TempDir::new().unwrap();
    let cfg = dir.path().join("config.toml");

    let mut child = rig_cmd(&cfg)
        .args(["--server", &url, "exec", "web", "--cols", "120", "--rows", "40"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(b"uname -a\n").await.unwrap();
    }
    let out = child.wait_with_output().await.unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("welcome> "), "stdout={stdout}");
    assert!(stdout.contains("echo:uname -a"), "stdout={stdout}");
    assert_eq!(
        *st.terminal_seen_cols_rows.lock().unwrap(),
        Some((Some(120), Some(40)))
    );
}

#[tokio::test]
async fn apps_lifecycle_domains_deploy_attach_and_pagination() {
    let (url, st) = start_phase2_server().await;
    let dir = TempDir::new().unwrap();
    let cfg = dir.path().join("config.toml");

    // apps create / set / rm
    let out = rig_cmd(&cfg)
        .args([
            "--server", &url, "--json", "apps", "create", "worker", "--project", "Alpha",
            "--image", "alpine:3", "--port", "8080", "--env", "FOO=bar",
        ])
        .output()
        .await
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let created: App = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(created.name, "worker");
    assert_eq!(created.port, 8080);

    let out = rig_cmd(&cfg)
        .args(["--server", &url, "--json", "apps", "set", "web", "--port", "9000", "--memory", "512"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    let updated: App = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(updated.port, 9000);

    // apps rm requires --yes in non-TTY, succeeds with --yes
    let out = rig_cmd(&cfg)
        .args(["--server", &url, "apps", "rm", "web"])
        .output()
        .await
        .unwrap();
    assert!(!out.status.success());
    let out = rig_cmd(&cfg)
        .args(["--server", &url, "--yes", "--json", "apps", "rm", "web"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());

    // start, restart, deployments cancel, deployments pagination
    let out = rig_cmd(&cfg)
        .args(["--server", &url, "--json", "start", "web", "--wait-healthy", "--timeout", "30s"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["status"], "running");
    assert_eq!(v["domains"][0], "web.example.com");

    let out = rig_cmd(&cfg)
        .args(["--server", &url, "--json", "restart", "web", "--no-follow"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());

    let out = rig_cmd(&cfg)
        .args(["--server", &url, "--json", "deployments", "cancel", DEP_ACTIVE])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    let dep: Deployment = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(dep.status, DeploymentStatus::Cancelled);

    let out = rig_cmd(&cfg)
        .args(["--server", &url, "--json", "deployments", "ls", "web", "--limit", "2", "--before", DEP_ACTIVE])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    let deps: Vec<Deployment> = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(deps.len(), 2);
    let q = st.last_deploy_query.lock().unwrap().clone().unwrap();
    assert_eq!(q.get("limit").map(String::as_str), Some("2"));
    assert_eq!(q.get("before").map(String::as_str), Some(DEP_ACTIVE));

    // 409 deploy conflict + --attach follows active deployment cleanly!
    *st.conflict_deploy.lock().unwrap() = true;
    let out = rig_cmd(&cfg)
        .args(["--server", &url, "deploy", "web"])
        .output()
        .await
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--attach"));

    let out = rig_cmd(&cfg)
        .args(["--server", &url, "deploy", "web", "--attach"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("Live at https://web.example.com"));
    *st.conflict_deploy.lock().unwrap() = false;

    // domains ls / add / rm
    let out = rig_cmd(&cfg)
        .args(["--server", &url, "--json", "domains", "add", "web", "app2.example.com", "--tls", "internal", "--access", "lan"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    let out = rig_cmd(&cfg)
        .args(["--server", &url, "-o", "name", "domains", "ls", "web"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    let names = String::from_utf8_lossy(&out.stdout);
    assert!(names.contains("web.example.com") && names.contains("app2.example.com"));

    let out = rig_cmd(&cfg)
        .args(["--server", &url, "--yes", "--json", "domains", "rm", "web", "app2.example.com"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
}

#[tokio::test]
async fn env_import_pull_diff_stdin_and_logs_filtering() {
    let (url, st) = start_phase2_server().await;
    let dir = TempDir::new().unwrap();
    let cfg = dir.path().join("config.toml");

    // env set KEY=- reads from stdin
    let mut child = rig_cmd(&cfg)
        .args(["--server", &url, "--json", "env", "set", "web", "FROM_STDIN=-", "--plain"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(b"piped-value\n").await.unwrap();
    }
    let out = child.wait_with_output().await.unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["set"][0]["value"], "piped-value");

    // env diff against local .env
    let env_file = dir.path().join(".env");
    std::fs::write(&env_file, "PLAIN_K=new-val\nLOCAL_NEW=123\n").unwrap();
    let out = rig_cmd(&cfg)
        .args(["--server", &url, "--json", "env", "diff", "web", env_file.to_str().unwrap()])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    let diffs: Vec<Value> = serde_json::from_slice(&out.stdout).unwrap();
    assert!(diffs.iter().any(|d| d["key"] == "PLAIN_K" && d["kind"] == "changed"));
    assert!(diffs.iter().any(|d| d["key"] == "LOCAL_NEW" && d["kind"] == "added"));

    // env import --plain --replace
    let out = rig_cmd(&cfg)
        .args([
            "--server", &url, "--json", "env", "import", "web", env_file.to_str().unwrap(),
            "--plain", "--replace",
        ])
        .output()
        .await
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let imp: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(imp["imported"], 2);

    // env pull writes .env file
    let pulled = dir.path().join("pulled.env");
    let out = rig_cmd(&cfg)
        .args(["--server", &url, "env", "pull", "web", "-f", pulled.to_str().unwrap()])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    let pulled_text = std::fs::read_to_string(&pulled).unwrap();
    assert!(pulled_text.contains("PLAIN_K=new-val"));
    assert!(pulled_text.contains("LOCAL_NEW=123"));

    // logs --tail --no-follow --since --timestamps --grep
    let out = rig_cmd(&cfg)
        .args([
            "--server", &url, "logs", "web", "--tail", "10", "--no-follow",
            "--since", "10m", "--timestamps", "--grep", "warn",
        ])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    let err_out = String::from_utf8_lossy(&out.stderr);
    assert!(err_out.contains("2026-11-22T12:00:02Z WARN: slow query"), "stderr={err_out}");
    let q = st.last_log_query.lock().unwrap().clone().unwrap();
    assert_eq!(q.get("tail").map(String::as_str), Some("10"));
    assert_eq!(q.get("follow").map(String::as_str), Some("false"));
    assert_eq!(q.get("since").map(String::as_str), Some("10m"));
    assert_eq!(q.get("timestamps").map(String::as_str), Some("true"));
    assert_eq!(q.get("grep").map(String::as_str), Some("warn"));
}

#[tokio::test]
async fn db_backup_metrics_top_tokens_members_audit_alerts_webhook_github() {
    let (url, _st) = start_phase2_server().await;
    let dir = TempDir::new().unwrap();
    let cfg = dir.path().join("config.toml");

    // db engines / create / show / start / stop / url / connect --print / rm
    for args in [
        vec!["--server", &url, "--json", "db", "engines"],
        vec!["--server", &url, "--json", "db", "create", "analytics", "--project", "Alpha", "--engine", "postgres", "--wait"],
        vec!["--server", &url, "--json", "db", "show", "pg"],
        vec!["--server", &url, "--json", "db", "start", "pg", "--wait"],
        vec!["--server", &url, "--json", "db", "stop", "pg"],
        vec!["--server", &url, "db", "url", "pg"],
        vec!["--server", &url, "db", "connect", "pg", "--print"],
        vec!["--server", &url, "--yes", "--json", "db", "rm", "pg"],
    ] {
        let out = rig_cmd(&cfg).args(&args).output().await.unwrap();
        assert!(
            out.status.success(),
            "args={args:?}\nstderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    // backup ls / create / download / restore / rm
    let dump_path = dir.path().join("dump.sql");
    for args in [
        vec!["--server", &url, "--json", "backup", "ls", "pg"],
        vec!["--server", &url, "--json", "backup", "create", "pg", "--wait"],
        vec!["--server", &url, "--json", "backup", "download", BK_ID, "-f", dump_path.to_str().unwrap()],
        vec!["--server", &url, "--yes", "--json", "backup", "restore", BK_ID],
        vec!["--server", &url, "--yes", "--json", "backup", "rm", BK_ID],
    ] {
        let out = rig_cmd(&cfg).args(&args).output().await.unwrap();
        assert!(
            out.status.success(),
            "args={args:?}\nstderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    assert_eq!(std::fs::read_to_string(&dump_path).unwrap(), "-- pg dump payload\n");

    // metrics (host, app, db) + top --once
    for args in [
        vec!["--server", &url, "metrics", "--range", "6h"],
        vec!["--server", &url, "metrics", "web", "--range", "24h"],
        vec!["--server", &url, "metrics", "db:pg"],
        vec!["--server", &url, "top", "--once"],
    ] {
        let out = rig_cmd(&cfg).args(&args).output().await.unwrap();
        assert!(
            out.status.success(),
            "args={args:?}\nstderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    // tokens, projects rename/rm, members, audit, alerts, webhook, github
    for args in [
        vec!["--server", &url, "--json", "tokens", "ls"],
        vec!["--server", &url, "--json", "tokens", "create", "ci", "--role", "developer"],
        vec!["--server", &url, "--yes", "--json", "tokens", "revoke", "tok-new"],
        vec!["--server", &url, "--json", "projects", "rename", "Alpha", "Alpha2"],
        vec!["--server", &url, "--yes", "--json", "projects", "rm", "Beta"],
        vec!["--server", &url, "--json", "members", "ls"],
        vec!["--server", &url, "--json", "members", "add", "Bob", "bob@example.com", "--role", "viewer"],
        vec!["--server", &url, "--json", "members", "role", "alice@example.com", "admin"],
        vec!["--server", &url, "--yes", "--json", "members", "reset-password", "alice@example.com"],
        vec!["--server", &url, "--yes", "--json", "members", "rm", "alice@example.com"],
        vec!["--server", &url, "--json", "audit", "--action", "deploy", "--limit", "1"],
        vec!["--server", &url, "--json", "alerts", "ls"],
        vec!["--server", &url, "--json", "alerts", "add", "pager", "https://example.com/hook", "--events", "deploy_failed,app_down"],
        vec!["--server", &url, "--json", "alerts", "set", "ops", "--disable"],
        vec!["--server", &url, "--json", "alerts", "test", "ops"],
        vec!["--server", &url, "--yes", "--json", "alerts", "rm", "ops"],
        vec!["--server", &url, "--json", "webhook", "show", "web"],
        vec!["--server", &url, "--yes", "--json", "webhook", "rotate", "web"],
        vec!["--server", &url, "--json", "github", "status"],
        vec!["--server", &url, "--json", "github", "repos"],
        vec!["--server", &url, "--json", "github", "branches", "acme/web"],
    ] {
        let out = rig_cmd(&cfg).args(&args).output().await.unwrap();
        assert!(
            out.status.success(),
            "args={args:?}\nstderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[tokio::test]
async fn rig_toml_linking_context_ux_yaml_verbose_skew_and_logout_revoke() {
    let (url, st) = start_phase2_server().await;
    let dir = TempDir::new().unwrap();
    let cfg = dir.path().join("config.toml");

    // Login populates context metadata, then test `context current`, `context rename`, and `-o yaml`.
    let out = rig_cmd(&cfg)
        .args(["login", &url, "--name", "dev"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());

    let out = rig_cmd(&cfg)
        .env_remove("RIG_TOKEN")
        .args(["context", "current"])
        .output()
        .await
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "dev");

    let out = rig_cmd(&cfg)
        .env_remove("RIG_TOKEN")
        .args(["context", "rename", "dev", "prod"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());

    let out = rig_cmd(&cfg)
        .env_remove("RIG_TOKEN")
        .args(["-o", "yaml", "context"])
        .output()
        .await
        .unwrap();
    let yaml = String::from_utf8_lossy(&out.stdout);
    assert!(yaml.contains("name: prod"), "yaml={yaml}");
    assert!(yaml.contains("user: owner@example.com"), "yaml={yaml}");
    assert!(yaml.contains("org: Acme"), "yaml={yaml}");

    // `rig link web` writes rig.toml, and running from a nested subdir omits `<app>`.
    let repo_dir = dir.path().join("repo");
    let nested = repo_dir.join("src/nested");
    std::fs::create_dir_all(&nested).unwrap();

    let out = rig_cmd(&cfg)
        .env_remove("RIG_TOKEN")
        .current_dir(&repo_dir)
        .args(["link", "web"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(repo_dir.join("rig.toml").is_file());

    let out = rig_cmd(&cfg)
        .env_remove("RIG_TOKEN")
        .current_dir(&nested)
        .args(["--json", "apps", "show"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let app: App = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(app.id, APP_ID);

    let out = rig_cmd(&cfg)
        .env_remove("RIG_TOKEN")
        .current_dir(&nested)
        .args(["open", "--print"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "https://web.example.com"
    );

    // Verbose logs HTTP method/status/timing on stderr without leaking token,
    // and version-skew warning fires when server version != CLI version.
    *st.server_version.lock().unwrap() = "9.9.9".into();
    let out = rig_cmd(&cfg)
        .env_remove("RIG_TOKEN")
        .args(["-v", "status"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("rig: debug: GET /api/v1/"), "stderr={stderr}");
    assert!(stderr.contains("differs from server version 9.9.9"), "stderr={stderr}");
    assert!(!stderr.contains(TOKEN), "token leaked in verbose output!");

    // `rig logout --revoke` revokes the matching token on the server and removes context.
    let out = rig_cmd(&cfg)
        .env_remove("RIG_TOKEN")
        .args(["--json", "logout", "--revoke"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["revoked"], true);
    assert_eq!(v["revoked_token_id"], "tok-current");
    assert!(st.tokens.lock().unwrap().is_empty());
}

#[tokio::test]
async fn mcp_phase2_tools_end_to_end() {
    let (url, _st) = start_phase2_server().await;
    let dir = TempDir::new().unwrap();
    let cfg = dir.path().join("config.toml");

    let mut child = rig_cmd(&cfg)
        .args(["--server", &url, "mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let calls = [
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"app_logs","arguments":{"app":"web","tail":50,"grep":"warn"}}}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"rollback_app","arguments":{"app":"web","wait":false}}}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_metrics","arguments":{"target":"web","range":"6h"}}}),
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"list_domains","arguments":{"app":"web"}}}),
        json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"add_domain","arguments":{"app":"web","host":"mcp.example.com","tls_mode":"internal"}}}),
        json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"remove_domain","arguments":{"app":"web","domain":"mcp.example.com"}}}),
        json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"create_database","arguments":{"project":"Alpha","name":"cache","engine":"redis"}}}),
        json!({"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"list_backups","arguments":{"database":"pg"}}}),
        json!({"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"create_backup","arguments":{"database":"pg"}}}),
        json!({"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"start_app","arguments":{"app":"web","wait":false}}}),
        json!({"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"restart_app","arguments":{"app":"web","wait":false}}}),
        json!({"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"cancel_deployment","arguments":{"deployment_id":DEP_ACTIVE}}}),
    ];

    {
        let mut stdin = child.stdin.take().unwrap();
        for c in &calls {
            stdin
                .write_all(format!("{c}\n").as_bytes())
                .await
                .unwrap();
        }
    }

    let out = child.wait_with_output().await.unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let replies: HashMap<i64, Value> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| {
            let v: Value = serde_json::from_str(l).unwrap();
            (v["id"].as_i64().unwrap(), v)
        })
        .collect();

    assert_eq!(replies.len(), calls.len());
    for id in 1..=calls.len() as i64 {
        let r = &replies[&id];
        assert_eq!(r["result"]["isError"], false, "tool id {id} failed: {r}");
    }
    assert_eq!(
        replies[&1]["result"]["structuredContent"]["logs"]["untrusted_log_output"],
        true
    );
    assert_eq!(replies[&2]["result"]["structuredContent"]["status"], "queued");
    assert_eq!(replies[&3]["result"]["structuredContent"]["range"], "6h");
    assert_eq!(replies[&5]["result"]["structuredContent"]["host"], "mcp.example.com");
    assert_eq!(replies[&6]["result"]["structuredContent"]["deleted"], "mcp.example.com");
    assert_eq!(replies[&12]["result"]["structuredContent"]["status"], "cancelled");
}
