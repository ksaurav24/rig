//! HTTP routes for discovering and adopting unmanaged Docker/Podman containers (`rig adopt`).

use axum::{Json, extract::State, http::StatusCode};
use rig_api::{
    AdoptExecuteRequest, AdoptExecuteResult, AdoptPreview, AdoptPreviewRequest,
    DiscoveredContainer, ErrorBody,
};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{
    adopt,
    auth::{Admin, Auth},
    error::{ApiError, ApiResult},
    state::AppState,
};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_adoptable_containers))
        .routes(routes!(preview_adopt))
        .routes(routes!(execute_adopt))
}

/// Inspect unmanaged Docker/Podman containers on this server that can be adopted into Rig.
/// Requires Admin role; suspected secret environment variables are never returned unmasked.
#[utoipa::path(
    get,
    path = "/api/v1/adopt/containers",
    tag = "adopt",
    responses((status = 200, body = Vec<DiscoveredContainer>), (status = 503, body = ErrorBody))
)]
async fn list_adoptable_containers(
    State(state): State<AppState>,
    _auth: Auth<Admin>,
) -> ApiResult<Json<Vec<DiscoveredContainer>>> {
    let containers = adopt::discover_containers(&state)
        .await
        .map_err(|e| ApiError::Unavailable(format!("discovering containers: {e:#}")))?;
    Ok(Json(containers))
}

/// Generate a dry-run preview (including `rig.toml` and Podman Quadlet `.container`/`.network` units)
/// before modifying anything on disk, in SQLite, or in the container engine.
#[utoipa::path(
    post,
    path = "/api/v1/adopt/preview",
    tag = "adopt",
    request_body = AdoptPreviewRequest,
    responses(
        (status = 200, body = AdoptPreview),
        (status = 404, body = ErrorBody),
        (status = 422, body = ErrorBody)
    )
)]
async fn preview_adopt(
    State(state): State<AppState>,
    auth: Auth<Admin>,
    Json(req): Json<AdoptPreviewRequest>,
) -> ApiResult<Json<AdoptPreview>> {
    let preview = adopt::preview_adopt(&state, &auth.org_id, &req).await?;
    Ok(Json(preview))
}

/// Adopt an external container into Rig with preserved mounts, vault-encrypted secrets,
/// health-checked zero-downtime (or rollback-protected) cutover, and Caddy proxy routing.
#[utoipa::path(
    post,
    path = "/api/v1/adopt/execute",
    tag = "adopt",
    request_body = AdoptExecuteRequest,
    responses(
        (status = 201, body = AdoptExecuteResult),
        (status = 404, body = ErrorBody),
        (status = 409, body = ErrorBody),
        (status = 422, body = ErrorBody),
        (status = 503, body = ErrorBody)
    )
)]
async fn execute_adopt(
    State(state): State<AppState>,
    auth: Auth<Admin>,
    Json(req): Json<AdoptExecuteRequest>,
) -> ApiResult<(StatusCode, Json<AdoptExecuteResult>)> {
    let res = adopt::execute_adopt(&state, &auth, &req).await?;
    Ok((StatusCode::CREATED, Json(res)))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        net::SocketAddr,
        path::Path,
        sync::{Arc, Mutex},
    };

    use rig_api::{
        AdoptExecuteRequest, AdoptExecuteResult, AdoptPreview, AdoptPreviewRequest,
        BinaryDeployRequest, CheckBlobsRequest, CheckBlobsResponse, ContextFileEntry,
        DiscoveredContainer, LocalSourceDeployRequest, StoredBlob, StoredBlobBatch,
    };
    use sha2::{Digest, Sha256};

    use crate::{
        proxy::Proxy,
        runtime::{
            BuildSpec, ByteReader, ByteWriter, ContainerSpec, ContainerState, EngineInfo,
            ExecOutcome, Health, InspectedContainer, InspectedMount, InspectedPort, LogOptions,
            LogSink, LogStream, PlanSpec, Runtime, Shell, Started, Usage,
        },
        state::{AppState, Config, RuntimeCaches},
    };

    #[derive(Default)]
    struct MockEngineState {
        external: Vec<InspectedContainer>,
        loaded_images: Vec<String>,
        started_containers: Vec<String>,
        stopped_containers: Vec<String>,
        removed_containers: Vec<String>,
        removed_volumes: Vec<String>,
        fail_health: bool,
        live_port: u16,
    }

    struct MockRuntime(Arc<Mutex<MockEngineState>>);

    #[async_trait::async_trait]
    impl Runtime for MockRuntime {
        async fn info(&self) -> anyhow::Result<EngineInfo> {
            Ok(EngineInfo {
                name: "podman".into(),
                version: "5.2.0".into(),
            })
        }
        async fn pull(&self, _: &str, _: &LogSink) -> anyhow::Result<()> {
            Ok(())
        }
        async fn build(&self, _: BuildSpec, _: &LogSink) -> anyhow::Result<()> {
            Ok(())
        }
        async fn build_plan(&self, _: PlanSpec, _: &LogSink) -> anyhow::Result<()> {
            Ok(())
        }
        async fn load_image(&self, archive_path: &Path, _: &LogSink) -> anyhow::Result<()> {
            let bytes = std::fs::read(archive_path)?;
            assert!(!bytes.is_empty());
            self.0
                .lock()
                .unwrap()
                .loaded_images
                .push(archive_path.display().to_string());
            Ok(())
        }
        async fn list_external_containers(&self) -> anyhow::Result<Vec<InspectedContainer>> {
            Ok(self.0.lock().unwrap().external.clone())
        }
        async fn inspect_container_full(
            &self,
            id: &str,
        ) -> anyhow::Result<Option<InspectedContainer>> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .external
                .iter()
                .find(|c| c.id == id || c.name == id)
                .cloned())
        }
        async fn run(&self, spec: &ContainerSpec) -> anyhow::Result<Started> {
            let mut s = self.0.lock().unwrap();
            let cid = format!("adopted-{}", spec.name);
            s.started_containers.push(cid.clone());
            Ok(Started {
                id: cid,
                host_port: s.live_port,
            })
        }
        async fn state(&self, id: &str) -> anyhow::Result<ContainerState> {
            let s = self.0.lock().unwrap();
            if s.fail_health && id.starts_with("adopted-") {
                return Ok(ContainerState::Exited(Some(1)));
            }
            Ok(ContainerState::Running)
        }
        async fn health(&self, _: &str) -> anyhow::Result<Option<Health>> {
            Ok(None)
        }
        async fn start(&self, id: &str) -> anyhow::Result<()> {
            self.0.lock().unwrap().started_containers.push(id.to_string());
            Ok(())
        }
        async fn stop(&self, id: &str, _: u32) -> anyhow::Result<()> {
            self.0.lock().unwrap().stopped_containers.push(id.to_string());
            Ok(())
        }
        async fn remove(&self, id: &str) -> anyhow::Result<()> {
            self.0.lock().unwrap().removed_containers.push(id.to_string());
            Ok(())
        }
        fn logs_with(&self, _: &str, _: LogOptions) -> LogStream {
            Box::pin(futures_util::stream::empty())
        }
        async fn list_by_label(&self, _: &str, _: &str) -> anyhow::Result<Vec<String>> {
            Ok(vec![])
        }
        async fn remove_volume(&self, name: &str) -> anyhow::Result<()> {
            self.0.lock().unwrap().removed_volumes.push(name.to_string());
            Ok(())
        }
        async fn remove_network(&self, _: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn shell(&self, _: &str, _: u16, _: u16) -> anyhow::Result<Shell> {
            anyhow::bail!("unused")
        }
        async fn image_id(&self, tag: &str) -> anyhow::Result<Option<String>> {
            Ok(Some(format!("sha256:loaded-{tag}")))
        }
        async fn usage(&self, _: &str) -> anyhow::Result<Option<Usage>> {
            Ok(None)
        }
        async fn resize_shell(&self, _: &str, _: u16, _: u16) -> anyhow::Result<()> {
            Ok(())
        }
        async fn exec(
            &self,
            _: &str,
            _: Vec<String>,
            _: Vec<String>,
            _: Option<ByteReader>,
            _: ByteWriter,
        ) -> anyhow::Result<ExecOutcome> {
            anyhow::bail!("unused")
        }
    }

    struct RecordingProxy(Arc<Mutex<Vec<String>>>);
    #[async_trait::async_trait]
    impl Proxy for RecordingProxy {
        async fn ready(&self) -> bool {
            true
        }
        async fn apply(
            &self,
            routes: &[crate::proxy::Route],
            _: Option<&rig_api::DashboardDomain>,
            _: &[String],
        ) -> anyhow::Result<()> {
            let mut hosts = self.0.lock().unwrap();
            *hosts = routes.iter().map(|r| r.host.clone()).collect();
            Ok(())
        }
        fn ports(&self) -> (u16, u16) {
            (80, 443)
        }
    }

    async fn setup_test_env(
        mock_state: Arc<Mutex<MockEngineState>>,
    ) -> (
        tempfile::TempDir,
        AppState,
        String,
        reqwest::Client,
        String,
        String,
        String,
        Arc<Mutex<Vec<String>>>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::state::open_db(&format!("sqlite://{}", dir.path().join("rig.db").display()))
            .await
            .unwrap();
        let vault = crate::crypto::Vault::open(dir.path()).unwrap();
        let proxy_hosts = Arc::new(Mutex::new(Vec::new()));
        let state = AppState {
            db: db.clone(),
            vault: vault.clone(),
            config: Arc::new(Config {
                data_dir: dir.path().to_path_buf(),
                secure_cookies: false,
                github_api: "https://api.github.com".into(),
                github_web: "https://github.com".into(),
                poll_interval: 0,
                allow_private_alerts: false,
                allow_private_git: false,
                enable_api_docs: false,
                mode: rig_api::ServerMode::Full,
            }),
            runtime: Arc::new(MockRuntime(mock_state)),
            proxy: Arc::new(RecordingProxy(Arc::clone(&proxy_hosts))),
            deploys: Arc::new(crate::deploy::Deployer::new().unwrap()),
            access: crate::access::DashboardAccess::default(),
            caches: Arc::new(RuntimeCaches::default()),
            events: Arc::new(crate::events::EventBus::new()),
        };

        let org_id = "01940000-0000-7000-8000-000000000001";
        let user_id = "01940000-0000-7000-8000-000000000002";
        let proj_id = "01940000-0000-7000-8000-000000000003";
        let app_id = "01940000-0000-7000-8000-000000000004";
        let raw_token = "rig_test_owner_token_1234567890";
        let token_hash = crate::crypto::hash_token(raw_token);

        sqlx::query("INSERT INTO organizations (id, name, slug) VALUES (?, 'Acme', 'acme')")
            .bind(org_id)
            .execute(&db)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO users (id, email, name, password_hash) VALUES (?, 'owner@acme.test', 'Owner', 'x')",
        )
        .bind(user_id)
        .execute(&db)
        .await
        .unwrap();
        sqlx::query("INSERT INTO memberships (org_id, user_id, role) VALUES (?, ?, 'owner')")
            .bind(org_id)
            .bind(user_id)
            .execute(&db)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO api_tokens (id, org_id, user_id, name, token_hash, prefix, role)
             VALUES ('t1', ?, ?, 'cli', ?, 'rig_test', 'owner')",
        )
        .bind(org_id)
        .bind(user_id)
        .bind(&token_hash)
        .execute(&db)
        .await
        .unwrap();
        sqlx::query("INSERT INTO projects (id, org_id, name, slug) VALUES (?, ?, 'Prod', 'prod')")
            .bind(proj_id)
            .bind(org_id)
            .execute(&db)
            .await
            .unwrap();
        let wh = vault.encrypt("whsec").unwrap();
        sqlx::query(
            "INSERT INTO apps (id, org_id, project_id, name, slug, source_kind, port, webhook_secret)
             VALUES (?, ?, ?, 'local-demo', 'local-demo', 'local', 8080, ?)",
        )
        .bind(app_id)
        .bind(org_id)
        .bind(proj_id)
        .bind(wh)
        .execute(&db)
        .await
        .unwrap();

        let router = crate::routes::app(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(
                listener,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await;
        });
        let base = format!("http://{addr}");
        let http = reqwest::Client::new();
        (
            dir,
            state,
            base,
            http,
            raw_token.to_string(),
            proj_id.to_string(),
            app_id.to_string(),
            proxy_hosts,
        )
    }

    #[tokio::test]
    async fn uploads_dedup_delta_source_and_binary_end_to_end() {
        let mock = Arc::new(Mutex::new(MockEngineState::default()));
        let (_dir, _state, base, http, token, _proj_id, app_id, _) =
            setup_test_env(Arc::clone(&mock)).await;

        // 1. Upload a static Linux ELF binary (zstd compressed) and verify dedup check.
        let mut elf = vec![0u8; 256];
        elf[0..4].copy_from_slice(b"\x7fELF");
        elf[4] = 2;
        elf[5] = 1;
        elf[18..20].copy_from_slice(&62u16.to_le_bytes()); // x86_64
        let bin_sha = hex::encode(Sha256::digest(&elf));

        let res = http
            .post(format!("{base}/api/v1/uploads/blobs/check"))
            .bearer_auth(&token)
            .json(&CheckBlobsRequest {
                hashes: vec![bin_sha.clone()],
            })
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        let check: CheckBlobsResponse = res.json().await.unwrap();
        assert_eq!(check.missing, vec![bin_sha.clone()]);

        let zst_bin = zstd::stream::encode_all(elf.as_slice(), 3).unwrap();
        let res = http
            .put(format!("{base}/api/v1/uploads/blobs/{bin_sha}"))
            .bearer_auth(&token)
            .header("content-type", "application/zstd")
            .body(zst_bin)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 201);
        let stored: StoredBlob = res.json().await.unwrap();
        assert_eq!(stored.size, 256);

        // Second check confirms blob is cached.
        let check2: CheckBlobsResponse = http
            .post(format!("{base}/api/v1/uploads/blobs/check"))
            .bearer_auth(&token)
            .json(&CheckBlobsRequest {
                hashes: vec![bin_sha.clone()],
            })
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(check2.present, vec![bin_sha.clone()]);
        assert!(check2.missing.is_empty());

        // Trigger Direct Binary deploy -> wraps in FROM scratch and loads image.
        let res = http
            .post(format!("{base}/api/v1/apps/{app_id}/deploy/binary"))
            .bearer_auth(&token)
            .json(&BinaryDeployRequest {
                sha256: bin_sha,
                filename: "my-server".into(),
                args: vec!["--port".into(), "8080".into()],
                base_image: None,
                port: Some(8080),
            })
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 202);
        assert_eq!(mock.lock().unwrap().loaded_images.len(), 1);

        let dep: rig_api::Deployment = res.json().await.unwrap();
        assert_eq!(dep.trigger, "local");
        let _ = http
            .post(format!("{base}/api/v1/deployments/{}/cancel", dep.id))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap();

        // 2. Upload delta batch zstd tarball and trigger local-source deploy.
        let df_bytes = b"FROM scratch\nCOPY hello /hello\n";
        let df_sha = hex::encode(Sha256::digest(df_bytes));
        let mut tar_bytes = Vec::new();
        {
            let mut b = tar::Builder::new(&mut tar_bytes);
            let mut hdr = tar::Header::new_gnu();
            hdr.set_size(df_bytes.len() as u64);
            hdr.set_mode(0o644);
            hdr.set_cksum();
            b.append_data(&mut hdr, &df_sha, df_bytes.as_slice()).unwrap();
            b.finish().unwrap();
        }
        let zst_batch = zstd::stream::encode_all(tar_bytes.as_slice(), 3).unwrap();
        let res = http
            .post(format!("{base}/api/v1/uploads/blobs/batch"))
            .bearer_auth(&token)
            .header("content-type", "application/zstd")
            .body(zst_batch)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 201);
        let batch: StoredBlobBatch = res.json().await.unwrap();
        assert_eq!(batch.stored, 1);

        let res = http
            .post(format!("{base}/api/v1/apps/{app_id}/deploy/local-source"))
            .bearer_auth(&token)
            .json(&LocalSourceDeployRequest {
                files: vec![ContextFileEntry {
                    path: "Dockerfile".into(),
                    sha256: df_sha,
                    mode: 0o644,
                    size: df_bytes.len() as u64,
                }],
                dockerfile: Some("Dockerfile".into()),
                label: Some("demo-dir".into()),
            })
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 202);
    }

    #[tokio::test]
    async fn adopt_discovery_dry_run_rollback_and_cutover_end_to_end() {
        // Bind a real loopback listener so `tcp_ready` health check passes when fail_health=false.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let live_port = listener.local_addr().unwrap().port();
        let _accept = tokio::spawn(async move {
            loop {
                if let Ok((_s, _)) = listener.accept().await {
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                }
            }
        });

        let bind_dir = tempfile::tempdir().unwrap();
        let bind_path = bind_dir.path().to_string_lossy().into_owned();

        let mock = Arc::new(Mutex::new(MockEngineState {
            external: vec![InspectedContainer {
                id: "ext-coolify-1234567890".into(),
                name: "billing-api".into(),
                state: "running".into(),
                image: "ghcr.io/acme/billing:1.4".into(),
                image_digest: Some("ghcr.io/acme/billing@sha256:deadbeef".into()),
                image_id: Some("sha256:img123".into()),
                labels: HashMap::from([
                    ("coolify.managed".into(), "true".into()),
                    ("coolify.projectName".into(), "acme-stack".into()),
                    (
                        "traefik.http.routers.billing.rule".into(),
                        "Host(`billing.acme.test`)".into(),
                    ),
                    ("traefik.http.routers.billing.tls".into(), "true".into()),
                    (
                        "traefik.http.services.billing.loadbalancer.server.port".into(),
                        "8080".into(),
                    ),
                ]),
                env: vec![
                    ("APP_ENV".into(), "production".into()),
                    ("STRIPE_SECRET_KEY".into(), "sk_live_topsecret".into()),
                ],
                ports: vec![InspectedPort {
                    container_port: 8080,
                    protocol: "tcp".into(),
                    host_ip: Some("0.0.0.0".into()),
                    host_port: Some(8080),
                }],
                mounts: vec![InspectedMount {
                    kind: "bind".into(),
                    source: bind_path.clone(),
                    destination: "/data".into(),
                    read_only: false,
                }],
                networks: vec!["coolify-net".into()],
                restart_policy: Some("unless-stopped".into()),
                memory_mb: Some(512),
                cpu_millis: Some(1000),
                command: Some(vec!["./billing".into()]),
                entrypoint: None,
                created_at: Some("2026-11-20T00:00:00Z".into()),
            }],
            live_port,
            ..Default::default()
        }));

        let (_dir, state, base, http, token, proj_id, _app_id, proxy_hosts) =
            setup_test_env(Arc::clone(&mock)).await;

        // 1. Discover containers: secret values MUST be masked.
        let res = http
            .get(format!("{base}/api/v1/adopt/containers"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        let found: Vec<DiscoveredContainer> = res.json().await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].origin, "coolify");
        assert_eq!(found[0].detected_port, Some(8080));
        assert_eq!(found[0].domains[0].host, "billing.acme.test");
        let secret_env = found[0]
            .env
            .iter()
            .find(|e| e.key == "STRIPE_SECRET_KEY")
            .unwrap();
        assert!(secret_env.is_secret_guess);
        assert!(
            secret_env.preview_value.is_none(),
            "suspected secret value must never be exposed in discovery"
        );

        // 2. Dry-run preview: generates rig.toml + Quadlet units without creating any app.
        let res = http
            .post(format!("{base}/api/v1/adopt/preview"))
            .bearer_auth(&token)
            .json(&AdoptPreviewRequest {
                container_id: "ext-coolify-1234567890".into(),
                project_id: Some(proj_id.clone()),
                app_name: None,
                port: None,
                health_path: None,
                domains: None,
                secret_keys: None,
                plain_keys: None,
            })
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        let preview: AdoptPreview = res.json().await.unwrap();
        assert!(preview.rig_toml.contains("billing.acme.test"));
        assert!(preview.quadlet_container.contains("[Container]"));
        assert!(preview.quadlet_network.contains("[Network]"));
        assert_eq!(preview.cutover_strategy, "stop-start-with-rollback");

        // 3. Automatic rollback when adopted container fails health check!
        mock.lock().unwrap().fail_health = true;
        let res = http
            .post(format!("{base}/api/v1/adopt/execute"))
            .bearer_auth(&token)
            .json(&AdoptExecuteRequest {
                container_id: "ext-coolify-1234567890".into(),
                project_id: proj_id.clone(),
                app_name: None,
                port: None,
                health_path: None,
                domains: None,
                secret_keys: None,
                plain_keys: None,
                stop_old: true,
                remove_old: false,
            })
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 503);
        {
            let m = mock.lock().unwrap();
            assert!(m.stopped_containers.contains(&"ext-coolify-1234567890".to_string()));
            assert!(m.started_containers.contains(&"ext-coolify-1234567890".to_string()));
            assert!(m.removed_containers.iter().any(|c| c.starts_with("adopted-")));
        }
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM apps WHERE name = 'billing-api'")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(count, 0);

        // 4. Successful cutover!
        mock.lock().unwrap().fail_health = false;
        let res = http
            .post(format!("{base}/api/v1/adopt/execute"))
            .bearer_auth(&token)
            .json(&AdoptExecuteRequest {
                container_id: "ext-coolify-1234567890".into(),
                project_id: proj_id.clone(),
                app_name: None,
                port: None,
                health_path: None,
                domains: None,
                secret_keys: None,
                plain_keys: None,
                stop_old: true,
                remove_old: false,
            })
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 201);
        let adopted: AdoptExecuteResult = res.json().await.unwrap();
        assert_eq!(adopted.app.name, "billing-api");
        assert_eq!(adopted.secrets_encrypted, 1);
        assert_eq!(adopted.domains_registered, vec!["billing.acme.test"]);
        assert!(adopted.old_container_stopped);
        assert!(!adopted.old_container_removed);
        assert!(proxy_hosts.lock().unwrap().contains(&"billing.acme.test".to_string()));

        // Verify secret was encrypted into Rig's vault and decrypts cleanly.
        let enc: String = sqlx::query_scalar(
            "SELECT value_enc FROM env_vars WHERE app_id = ? AND key = 'STRIPE_SECRET_KEY' AND is_secret = 1",
        )
        .bind(&adopted.app.id)
        .fetch_one(&state.db)
        .await
        .unwrap();
        assert_eq!(state.vault.decrypt(&enc).unwrap(), "sk_live_topsecret");

        // 5. Deleting the adopted app must NEVER delete its preserved host bind mount!
        let res = http
            .delete(format!("{base}/api/v1/apps/{}", adopted.app.id))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 204);
        assert!(
            mock.lock().unwrap().removed_volumes.is_empty(),
            "adopted bind mounts (owned=0) must never be removed when deleting the app"
        );
    }
}
