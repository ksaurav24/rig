//! End-to-end integration tests for `rig up`, `rig adopt`, and their MCP tools.

use std::{
    collections::HashSet,
    convert::Infallible,
    process::Stdio,
    sync::{Arc, Mutex},
};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{
        IntoResponse,
        sse::{Event, Sse},
    },
    routing::{get, post, put},
};
use rig_api::{
    Access, AddDomainRequest, AdoptEnvPreview, AdoptExecuteRequest, AdoptExecuteResult,
    AdoptPreview, AdoptPreviewRequest, App, AppStatus, BinaryDeployRequest, CheckBlobsRequest,
    CheckBlobsResponse, Deployment, DeploymentEndEvent, DeploymentStatus, DiscoveredContainer,
    DiscoveredDomain, DiscoveredEnvVar, DiscoveredMount, DiscoveredPort, LocalSourceDeployRequest,
    LogLine, Project, SourceKind, StoredBlob, StoredBlobBatch, TlsMode,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::{io::AsyncWriteExt, net::TcpListener, process::Command};

const TOKEN: &str = "rig_up_adopt_test_token";
const PROJ_ID: &str = "01940000-0000-7000-8000-000000000101";
const APP_ID: &str = "01940000-0000-7000-8000-000000000202";
const DEP_ID: &str = "01940000-0000-7000-8000-000000000303";

#[derive(Default)]
struct MockServer {
    blobs: Mutex<HashSet<String>>,
    last_source_files: Mutex<Vec<String>>,
    last_binary_filename: Mutex<Option<String>>,
    adopt_executed: Mutex<bool>,
}

fn sample_app() -> App {
    App {
        id: APP_ID.into(),
        project_id: PROJ_ID.into(),
        project_name: "Prod".into(),
        name: "demo".into(),
        slug: "demo".into(),
        source_kind: SourceKind::Local,
        image: None,
        git_url: None,
        git_branch: None,
        dockerfile_path: "Dockerfile".into(),
        port: 8080,
        memory_mb: Some(512),
        cpu_millis: None,
        health_path: Some("/health".into()),
        status: AppStatus::Running,
        current_deployment_id: Some(DEP_ID.into()),
        host_port: Some(25000),
        domains: vec!["demo.example.com".into()],
        volumes: vec![],
        last_crash: None,
        auto_deploy: false,
        created_at: "2026-11-24T00:00:00Z".into(),
        updated_at: "2026-11-24T00:00:00Z".into(),
    }
}

fn sample_deployment(trigger: &str) -> Deployment {
    Deployment {
        id: DEP_ID.into(),
        app_id: APP_ID.into(),
        status: DeploymentStatus::Running,
        trigger: trigger.into(),
        image_ref: Some("rig/demo:up-1".into()),
        commit_sha: None,
        commit_msg: Some("Local deploy".into()),
        error: None,
        created_at: "2026-11-24T00:00:00Z".into(),
        started_at: Some("2026-11-24T00:00:01Z".into()),
        finished_at: Some("2026-11-24T00:00:02Z".into()),
    }
}

fn sample_discovered() -> DiscoveredContainer {
    DiscoveredContainer {
        id: "docker-ext-9999".into(),
        name: "legacy-web".into(),
        state: "running".into(),
        image: "nginx:1.27".into(),
        image_digest: Some("docker.io/library/nginx@sha256:aaa".into()),
        image_id: Some("sha256:bbb".into()),
        origin: "compose".into(),
        stack: Some("legacy-stack".into()),
        service: Some("web".into()),
        ports: vec![DiscoveredPort {
            container_port: 80,
            protocol: "tcp".into(),
            host_ip: Some("0.0.0.0".into()),
            host_port: Some(8080),
        }],
        detected_port: Some(80),
        mounts: vec![DiscoveredMount {
            kind: "bind".into(),
            source: "/srv/www".into(),
            destination: "/usr/share/nginx/html".into(),
            read_only: true,
        }],
        networks: vec!["legacy-stack_default".into()],
        env: vec![DiscoveredEnvVar {
            key: "API_SECRET".into(),
            preview_value: None,
            is_secret_guess: true,
            secret_reason: Some("key contains SECRET".into()),
        }],
        domains: vec![DiscoveredDomain {
            host: "legacy.example.com".into(),
            tls_mode: TlsMode::Auto,
            source: "traefik".into(),
            service_port: Some(80),
        }],
        restart_policy: Some("unless-stopped".into()),
        memory_mb: Some(256),
        cpu_millis: Some(500),
        command: None,
        entrypoint: None,
        health_path: Some("/healthz".into()),
        created_at: Some("2026-11-20T00:00:00Z".into()),
    }
}

async fn start_mock_server() -> (String, Arc<MockServer>) {
    let state = Arc::new(MockServer::default());
    let app = Router::new()
        .route(
            "/api/v1/projects",
            get(|| async {
                Json(vec![Project {
                    id: PROJ_ID.into(),
                    name: "Prod".into(),
                    slug: "prod".into(),
                    description: "".into(),
                    app_count: 1,
                    created_at: "2026-11-20T00:00:00Z".into(),
                }])
            }),
        )
        .route(
            "/api/v1/apps/resolve",
            get(|Query(_q): Query<std::collections::HashMap<String, String>>| async {
                Json(sample_app())
            }),
        )
        .route("/api/v1/apps/{id}", get(|| async { Json(sample_app()) }))
        .route(
            "/api/v1/uploads/blobs/check",
            post(
                |State(st): State<Arc<MockServer>>, Json(req): Json<CheckBlobsRequest>| async move {
                    let blobs = st.blobs.lock().unwrap();
                    let mut present = Vec::new();
                    let mut missing = Vec::new();
                    for h in req.hashes {
                        if blobs.contains(&h) {
                            present.push(h);
                        } else {
                            missing.push(h);
                        }
                    }
                    Json(CheckBlobsResponse { present, missing })
                },
            ),
        )
        .route(
            "/api/v1/uploads/blobs/{sha}",
            put(
                |State(st): State<Arc<MockServer>>, Path(sha): Path<String>, body: Bytes| async move {
                    let raw = zstd::stream::decode_all(body.as_ref()).unwrap();
                    st.blobs.lock().unwrap().insert(sha.clone());
                    (
                        StatusCode::CREATED,
                        Json(StoredBlob {
                            sha256: sha,
                            size: raw.len() as u64,
                        }),
                    )
                },
            ),
        )
        .route(
            "/api/v1/uploads/blobs/batch",
            post(|State(st): State<Arc<MockServer>>, body: Bytes| async move {
                let raw = zstd::stream::decode_all(body.as_ref()).unwrap();
                let mut archive = tar::Archive::new(raw.as_slice());
                let mut count = 0usize;
                let mut total = 0u64;
                for e in archive.entries().unwrap() {
                    let e = e.unwrap();
                    let name = e.path().unwrap().to_string_lossy().into_owned();
                    total += e.size();
                    st.blobs.lock().unwrap().insert(name);
                    count += 1;
                }
                (
                    StatusCode::CREATED,
                    Json(StoredBlobBatch {
                        stored: count,
                        bytes: total,
                    }),
                )
            }),
        )
        .route(
            "/api/v1/apps/{id}/deploy/local-source",
            post(
                |State(st): State<Arc<MockServer>>, Json(req): Json<LocalSourceDeployRequest>| async move {
                    *st.last_source_files.lock().unwrap() =
                        req.files.into_iter().map(|f| f.path).collect();
                    (StatusCode::ACCEPTED, Json(sample_deployment("local")))
                },
            ),
        )
        .route(
            "/api/v1/apps/{id}/deploy/binary",
            post(
                |State(st): State<Arc<MockServer>>, Json(req): Json<BinaryDeployRequest>| async move {
                    *st.last_binary_filename.lock().unwrap() = Some(req.filename);
                    (StatusCode::ACCEPTED, Json(sample_deployment("local")))
                },
            ),
        )
        .route(
            "/api/v1/deployments/{id}",
            get(|| async { Json(sample_deployment("local")) }),
        )
        .route(
            "/api/v1/deployments/{id}/logs",
            get(|| async {
                let line = serde_json::to_string(&LogLine {
                    seq: 1,
                    ts: "2026-11-24T00:00:01Z".into(),
                    stream: "deploy".into(),
                    line: "Live on 127.0.0.1:25000".into(),
                })
                .unwrap();
                let end = serde_json::to_string(&DeploymentEndEvent {
                    status: "running".into(),
                    domains: vec!["demo.example.com".into()],
                })
                .unwrap();
                let stream = futures_util::stream::iter(vec![
                    Ok::<Event, Infallible>(Event::default().event("log").id("1").data(line)),
                    Ok::<Event, Infallible>(Event::default().event("end").data(end)),
                ]);
                Sse::new(stream).into_response()
            }),
        )
        .route(
            "/api/v1/adopt/containers",
            get(|| async { Json(vec![sample_discovered()]) }),
        )
        .route(
            "/api/v1/adopt/preview",
            post(|Json(_req): Json<AdoptPreviewRequest>| async {
                Json(AdoptPreview {
                    container: sample_discovered(),
                    app_name: "web".into(),
                    project_id: Some(PROJ_ID.into()),
                    project_name: Some("Prod".into()),
                    image: "docker.io/library/nginx@sha256:aaa".into(),
                    port: 80,
                    health_path: Some("/healthz".into()),
                    memory_mb: Some(256),
                    cpu_millis: Some(500),
                    mounts: sample_discovered().mounts,
                    domains: vec![AddDomainRequest {
                        host: "legacy.example.com".into(),
                        tls_mode: TlsMode::Auto,
                        access: Access::Public,
                        port: None,
                    }],
                    env_keys: vec![AdoptEnvPreview {
                        key: "API_SECRET".into(),
                        is_secret: true,
                        secret_reason: Some("key contains SECRET".into()),
                    }],
                    rig_toml: "app = \"web\"\nport = 80\n".into(),
                    quadlet_container: "[Container]\nImage=nginx:1.27\n".into(),
                    quadlet_network: "[Network]\nNetworkName=rig-prod\n".into(),
                    cutover_strategy: "zero-downtime".into(),
                    warnings: vec![],
                })
            }),
        )
        .route(
            "/api/v1/adopt/execute",
            post(
                |State(st): State<Arc<MockServer>>, Json(_req): Json<AdoptExecuteRequest>| async move {
                    *st.adopt_executed.lock().unwrap() = true;
                    (
                        StatusCode::CREATED,
                        Json(AdoptExecuteResult {
                            app: sample_app(),
                            deployment: sample_deployment("adopt"),
                            old_container_id: "docker-ext-9999".into(),
                            old_container_stopped: true,
                            old_container_removed: false,
                            domains_registered: vec!["legacy.example.com".into()],
                            volumes_preserved: sample_discovered().mounts,
                            secrets_encrypted: 1,
                            rig_toml: "app = \"web\"\nport = 80\n".into(),
                            quadlet_container: "[Container]\nImage=nginx:1.27\n".into(),
                            quadlet_network: "[Network]\nNetworkName=rig-prod\n".into(),
                        }),
                    )
                },
            ),
        )
        .with_state(Arc::clone(&state));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}"), state)
}

fn rig_bin(cfg: &TempDir) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rig"));
    cmd.env("RIG_CONFIG", cfg.path().join("config.toml"))
        .env("RIG_TOKEN", TOKEN)
        .env("NO_COLOR", "1");
    cmd
}

#[tokio::test]
async fn rig_up_remote_build_dedup_and_direct_binary_mode() {
    let (url, state) = start_mock_server().await;
    let cfg = TempDir::new().unwrap();
    let proj = TempDir::new().unwrap();

    // Create a local project with .gitignore + .dockerignore + rig.toml.
    std::fs::create_dir(proj.path().join(".git")).unwrap();
    std::fs::write(proj.path().join(".gitignore"), "ignored.log\n").unwrap();
    std::fs::write(proj.path().join(".dockerignore"), "notes.txt\n").unwrap();
    std::fs::write(proj.path().join("Dockerfile"), "FROM scratch\n").unwrap();
    std::fs::write(proj.path().join("main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(proj.path().join("ignored.log"), "skip\n").unwrap();
    std::fs::write(proj.path().join("notes.txt"), "skip\n").unwrap();
    std::fs::write(proj.path().join("rig.toml"), "app = \"demo\"\n").unwrap();

    // 1. `rig up --remote-build --node <url> --json`
    let out = rig_bin(&cfg)
        .current_dir(proj.path())
        .args(["up", "--node", &url, "--remote-build", "--json"])
        .output()
        .await
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let files = state.last_source_files.lock().unwrap().clone();
    assert!(files.contains(&"Dockerfile".to_string()));
    assert!(files.contains(&"main.rs".to_string()));
    assert!(!files.contains(&"ignored.log".to_string()));
    assert!(!files.contains(&"notes.txt".to_string()));

    // 2. Direct binary mode: `rig up ./my-linux-app --node <url> --json`
    let bin_path = proj.path().join("my-linux-app");
    let mut elf = vec![0u8; 128];
    elf[0..4].copy_from_slice(b"\x7fELF");
    std::fs::write(&bin_path, &elf).unwrap();

    let out = rig_bin(&cfg)
        .current_dir(proj.path())
        .args([
            "up",
            bin_path.to_str().unwrap(),
            "--node",
            &url,
            "--app",
            "demo",
            "--json",
        ])
        .output()
        .await
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        state.last_binary_filename.lock().unwrap().as_deref(),
        Some("my-linux-app")
    );
}

#[tokio::test]
async fn rig_adopt_dry_run_export_execute_and_mcp_tools() {
    let (url, state) = start_mock_server().await;
    let cfg = TempDir::new().unwrap();
    let export_dir = TempDir::new().unwrap();

    // 1. Dry-run with Quadlet & rig.toml export (does NOT execute cutover).
    let out = rig_bin(&cfg)
        .args([
            "adopt",
            "legacy-web",
            "--node",
            &url,
            "--dry-run",
            "--export-quadlets",
            export_dir.path().to_str().unwrap(),
            "--write-rig-toml",
            export_dir.path().to_str().unwrap(),
        ])
        .output()
        .await
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!*state.adopt_executed.lock().unwrap());
    assert!(export_dir.path().join("rig-web.container").is_file());
    assert!(export_dir.path().join("rig-web.network").is_file());
    assert!(export_dir.path().join("rig.toml").is_file());

    // 2. Confirmed cutover (`--yes --json`).
    let out = rig_bin(&cfg)
        .args([
            "adopt",
            "legacy-web",
            "--node",
            &url,
            "--project",
            "Prod",
            "--yes",
            "--json",
        ])
        .output()
        .await
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(*state.adopt_executed.lock().unwrap());

    // 3. MCP tools: `discover_containers` and `preview_adopt`.
    let mut child = rig_bin(&cfg)
        .args(["--server", &url, "mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        let reqs = [
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"discover_containers","arguments":{}}}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"preview_adopt","arguments":{"container_id":"docker-ext-9999"}}}),
        ];
        for r in reqs {
            stdin
                .write_all(format!("{}\n", serde_json::to_string(&r).unwrap()).as_bytes())
                .await
                .unwrap();
        }
    }
    let out = child.wait_with_output().await.unwrap();
    assert!(out.status.success());
    let replies: Vec<Value> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(replies.len(), 2);
    assert_eq!(replies[0]["result"]["isError"], Value::Bool(false));
    assert_eq!(replies[1]["result"]["isError"], Value::Bool(false));
    let preview_str = replies[1]["result"]["content"][0]["text"].as_str().unwrap();
    assert!(preview_str.contains("quadlet_container"));
}
