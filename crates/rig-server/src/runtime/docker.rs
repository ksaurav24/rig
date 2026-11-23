//! [`Runtime`] over the Docker Engine API, which Podman also serves.

use std::{collections::HashMap, path::Path, time::Duration};

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use bollard::{
    API_DEFAULT_VERSION, Docker, body_stream,
    container::LogOutput,
    errors::Error as EngineError,
    exec::{CreateExecOptions, ResizeExecOptions, StartExecOptions, StartExecResults},
    models::{
        ContainerCreateBody, ContainerStateStatusEnum, ContainerStatsResponse, EndpointSettings,
        HostConfig, NetworkCreateRequest, NetworkingConfig, PortBinding, RestartPolicy,
        RestartPolicyNameEnum,
    },
    query_parameters::{
        BuildImageOptionsBuilder, CreateContainerOptionsBuilder, CreateImageOptionsBuilder,
        ListContainersOptionsBuilder, LogsOptionsBuilder, RemoveContainerOptionsBuilder,
        RemoveVolumeOptions, StatsOptionsBuilder, StopContainerOptionsBuilder,
    },
};
use futures_util::StreamExt;
use tokio_util::io::ReaderStream;

use super::{
    BuildSpec, ByteReader, ByteWriter, ContainerSpec, ContainerState, EngineInfo, ExecOutcome,
    Health, LogMsg, LogSink, LogStream, PlanSpec, Runtime, Shell, Started, Usage,
};

/// BuildKit used for plan builds on Podman, which has no BuildKit of its own.
const BUILDKIT_IMAGE: &str = "docker.io/moby/buildkit:v0.35.1";
/// Keeps BuildKit's layer cache between Podman builds.
const BUILDKIT_CACHE: &str = "rig-buildkit-cache";

/// How much stderr an exec keeps for its error message.
const STDERR_TAIL: usize = 4096;

/// Requests that return quickly.
const API_TIMEOUT_SECS: u64 = 60;
/// Pulls and builds stream for as long as the work takes.
const LONG_TIMEOUT: Duration = Duration::from_secs(60 * 60);

#[derive(Clone)]
pub struct DockerRuntime {
    api: Docker,
    long: Docker,
    /// Engine socket path, handed to the `docker buildx` CLI for plan builds.
    socket: String,
}

impl DockerRuntime {
    /// Connect to `socket`, or to the first engine socket that exists.
    ///
    /// Connecting is lazy: a missing or stopped engine surfaces on first use,
    /// so the dashboard can still start and report it.
    pub fn connect(socket: Option<&str>) -> Result<Self> {
        let path = match socket {
            Some(s) => s.trim_start_matches("unix://").to_string(),
            None => default_socket(),
        };
        let api = Docker::connect_with_unix(&path, API_TIMEOUT_SECS, API_DEFAULT_VERSION)
            .with_context(|| format!("connecting to engine socket {path}"))?;
        let long = api.clone().with_timeout(LONG_TIMEOUT);
        tracing::info!(socket = %path, "container engine socket");
        Ok(Self {
            api,
            long,
            socket: path,
        })
    }
}

impl DockerRuntime {
    /// Docker: its built-in BuildKit, through the `docker buildx` CLI.
    async fn build_plan_buildx(&self, spec: &PlanSpec, log: &LogSink) -> Result<()> {
        let mut cmd = tokio::process::Command::new("docker");
        cmd.env("DOCKER_HOST", format!("unix://{}", self.socket))
            .env("BUILDX_NO_DEFAULT_ATTESTATIONS", "1")
            .args([
                "buildx",
                "build",
                "--progress",
                "plain",
                "--provenance=false",
            ])
            .arg("--build-arg")
            .arg(format!("BUILDKIT_SYNTAX={}", spec.frontend))
            .arg("-f")
            .arg(spec.plan_dir.join("railpack-plan.json"))
            .arg("--output")
            .arg(format!("type=docker,name={}", spec.tag))
            .arg("--")
            .arg(&spec.context)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow!(
                    "building without a Dockerfile needs the docker CLI with buildx on the server"
                )
            } else {
                anyhow!(e).context("starting docker buildx")
            }
        })?;
        let out = child.stdout.take().context("no stdout")?;
        let err = child.stderr.take().context("no stderr")?;
        let (_, _, status) = tokio::join!(pump_lines(out, log), pump_lines(err, log), child.wait());
        let status = status.context("waiting for docker buildx")?;
        if !status.success() {
            bail!("the build failed ({status}). See the build log above");
        }
        Ok(())
    }

    /// Podman: a throwaway BuildKit container that writes an image archive,
    /// which is then loaded into the engine.
    async fn build_plan_buildkit(&self, spec: &PlanSpec, log: &LogSink) -> Result<()> {
        // Only permit a privileged BuildKit container when Podman is actually
        // running rootless (confined to an unprivileged user namespace). Under
        // rootful Podman, `privileged: true` would expose host root capabilities
        // to untrusted repository builds.
        let sys = self
            .api
            .info()
            .await
            .context("inspecting container engine security mode")?;
        let is_rootless = sys
            .security_options
            .as_deref()
            .unwrap_or_default()
            .iter()
            .any(|opt| opt.eq_ignore_ascii_case("name=rootless") || opt.eq_ignore_ascii_case("rootless"));
        if !is_rootless {
            bail!(
                "refusing to run a privileged BuildKit container on rootful Podman; \
                 switch to rootless Podman or add a Dockerfile to the repository"
            );
        }
        self.pull(BUILDKIT_IMAGE, log).await?;
        let out_dir = spec.work_dir.join("out");
        tokio::fs::create_dir_all(&out_dir).await?;
        let bind = |host: &Path, inner: &str, ro: bool| {
            format!("{}:{inner}{}", host.display(), if ro { ":ro" } else { "" })
        };
        let host = HostConfig {
            binds: Some(vec![
                bind(&spec.context, "/ctx", true),
                bind(&spec.plan_dir, "/plan", true),
                bind(&out_dir, "/out", false),
                format!("{BUILDKIT_CACHE}:/var/lib/buildkit"),
            ]),
            // BuildKit needs to create namespaces and mounts. Under rootless
            // Podman this is confined to the user's own namespace.
            privileged: Some(true),
            ..Default::default()
        };
        let name = qualify_image(&spec.tag);
        let cmd = vec![
            "build".to_string(),
            "--progress".into(),
            "plain".into(),
            "--local".into(),
            "context=/ctx".into(),
            "--local".into(),
            "dockerfile=/plan".into(),
            "--opt".into(),
            "filename=railpack-plan.json".into(),
            "--frontend".into(),
            "gateway.v0".into(),
            "--opt".into(),
            format!("source={}", spec.frontend),
            "--output".into(),
            format!("type=docker,name={name},dest=/out/image.tar"),
        ];
        let body = ContainerCreateBody {
            image: Some(BUILDKIT_IMAGE.into()),
            entrypoint: Some(vec!["buildctl-daemonless.sh".into()]),
            cmd: Some(cmd),
            host_config: Some(host),
            ..Default::default()
        };
        let id = self
            .api
            .create_container(
                None::<bollard::query_parameters::CreateContainerOptions>,
                body,
            )
            .await
            .context("creating the BuildKit container")?
            .id;
        let result = self.run_buildkit(&id, log).await;
        let _ = self
            .api
            .remove_container(
                &id,
                Some(
                    RemoveContainerOptionsBuilder::default()
                        .force(true)
                        .v(true)
                        .build(),
                ),
            )
            .await;
        result?;

        log.line("build", "Loading the image").await;
        let file = tokio::fs::File::open(out_dir.join("image.tar"))
            .await
            .context("the build wrote no image")?;
        let stream = ReaderStream::with_capacity(file, 64 * 1024);
        let mut load = self.long.import_image_stream(
            bollard::query_parameters::ImportImageOptions::default(),
            stream,
            None,
        );
        while let Some(item) = load.next().await {
            let info = item.context("loading the built image")?;
            if let Some(detail) = info.error_detail {
                bail!(
                    "loading the image failed: {}",
                    detail.message.unwrap_or_default()
                );
            }
        }
        Ok(())
    }

    async fn run_buildkit(&self, id: &str, log: &LogSink) -> Result<()> {
        self.api
            .start_container(id, None)
            .await
            .context("starting the BuildKit container")?;
        let opts = LogsOptionsBuilder::default()
            .stdout(true)
            .stderr(true)
            .follow(true)
            .tail("all")
            .build();
        let mut logs = self.long.logs(id, Some(opts));
        while let Some(Ok(chunk)) = logs.next().await {
            for line in String::from_utf8_lossy(chunk.as_ref()).lines() {
                log.line("build", line).await;
            }
        }
        let mut wait = self.long.wait_container(id, None);
        match wait.next().await {
            Some(Ok(r)) if r.status_code == 0 => Ok(()),
            Some(Ok(r)) => bail!(
                "the build failed (exit {}). See the build log above",
                r.status_code
            ),
            Some(Err(EngineError::DockerContainerWaitError { code, .. })) => {
                bail!("the build failed (exit {code}). See the build log above")
            }
            Some(Err(e)) => Err(e).context("waiting for the build"),
            None => Ok(()),
        }
    }
}

async fn pump_lines(r: impl tokio::io::AsyncRead + Unpin, log: &LogSink) {
    use tokio::io::AsyncBufReadExt;
    let mut lines = tokio::io::BufReader::new(r).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        log.line("build", line).await;
    }
}

fn default_socket() -> String {
    let mut candidates = vec!["/var/run/docker.sock".to_string()];
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        candidates.push(format!("{dir}/podman/podman.sock"));
    }
    candidates.push("/run/podman/podman.sock".into());
    candidates
        .iter()
        .find(|p| Path::new(p.as_str()).exists())
        .unwrap_or(&candidates[0])
        .clone()
}

fn is_not_found(e: &EngineError) -> bool {
    matches!(
        e,
        EngineError::DockerResponseServerError {
            status_code: 404,
            ..
        }
    )
}

fn is_status(e: &EngineError, code: u16) -> bool {
    matches!(e, EngineError::DockerResponseServerError { status_code, .. } if *status_code == code)
}

fn is_not_modified(e: &EngineError) -> bool {
    matches!(
        e,
        EngineError::DockerResponseServerError {
            status_code: 304,
            ..
        }
    )
}

#[async_trait]
impl Runtime for DockerRuntime {
    async fn info(&self) -> Result<EngineInfo> {
        let v = self
            .api
            .version()
            .await
            .context("engine is not reachable")?;
        let podman = v
            .components
            .iter()
            .flatten()
            .any(|c| c.name.to_lowercase().contains("podman"));
        Ok(EngineInfo {
            name: if podman { "podman" } else { "docker" }.into(),
            version: v.version.unwrap_or_default(),
        })
    }

    async fn pull(&self, image: &str, log: &LogSink) -> Result<()> {
        // Docker reads short names as Docker Hub ones; Podman refuses them
        // unless registries.conf lists search registries. Spelling the name
        // out behaves the same on both.
        let qualified = qualify_image(image);
        let opts = CreateImageOptionsBuilder::default()
            .from_image(qualified.as_str())
            .build();
        let mut stream = self.long.create_image(Some(opts), None, None);
        let mut last = String::new();
        while let Some(item) = stream.next().await {
            let info = item.with_context(|| format!("pulling {image}"))?;
            if let Some(detail) = info.error_detail {
                bail!("pulling {image}: {}", detail.message.unwrap_or_default());
            }
            // Layer progress repeats the same status many times a second;
            // only changes are worth a log line.
            let status = match (info.id, info.status) {
                (Some(id), Some(s)) => format!("{id}: {s}"),
                (None, Some(s)) => s,
                _ => continue,
            };
            if status != last && !status.contains("Downloading") && !status.contains("Extracting") {
                log.line("build", &status).await;
                last = status;
            }
        }
        Ok(())
    }

    async fn image_id(&self, image: &str) -> Result<Option<String>> {
        let qualified = qualify_image(image);
        for name in [image, qualified.as_str()] {
            match self.api.inspect_image(name).await {
                Ok(info) => return Ok(info.id),
                Err(EngineError::DockerResponseServerError {
                    status_code: 404, ..
                }) => {}
                Err(e) => return Err(e).context("inspecting the image"),
            }
            if name == qualified {
                break;
            }
        }
        Ok(None)
    }

    async fn build(&self, spec: BuildSpec, log: &LogSink) -> Result<()> {
        let context = spec.context.clone();
        let tarball = tokio::task::spawn_blocking(move || pack_context(&context))
            .await
            .context("packing build context")??;
        let file = tokio::fs::File::from_std(tarball);
        let body = ReaderStream::with_capacity(file, 64 * 1024)
            .take_while(|chunk| std::future::ready(chunk.is_ok()))
            .filter_map(|chunk| std::future::ready(chunk.ok()));

        let opts = BuildImageOptionsBuilder::default()
            .dockerfile(&spec.dockerfile)
            .t(&spec.tag)
            .labels(&spec.labels)
            .rm(true)
            .forcerm(true)
            .build();
        let mut stream = self.long.build_image(opts, None, Some(body_stream(body)));
        while let Some(item) = stream.next().await {
            let info = item.context("building image")?;
            if let Some(detail) = info.error_detail {
                bail!("build failed: {}", detail.message.unwrap_or_default());
            }
            if let Some(text) = info.stream {
                log.line("build", text).await;
            }
        }
        Ok(())
    }

    async fn build_plan(&self, spec: PlanSpec, log: &LogSink) -> Result<()> {
        let work = async {
            if self.info().await?.name == "podman" {
                self.build_plan_buildkit(&spec, log).await
            } else {
                self.build_plan_buildx(&spec, log).await
            }
        };
        tokio::time::timeout(LONG_TIMEOUT, work)
            .await
            .map_err(|_| {
                anyhow!(
                    "the build took longer than {} minutes",
                    LONG_TIMEOUT.as_secs() / 60
                )
            })?
    }

    async fn run(&self, spec: &ContainerSpec) -> Result<Started> {
        let port_key = format!("{}/tcp", spec.port);
        if let Some(network) = &spec.network {
            self.ensure_network(network).await?;
        }
        // Podman has no daemon to honour `unless-stopped` after a reboot; its
        // podman-restart.service only brings back `always` containers (on
        // Podman, the two policies behave the same otherwise). Rig removes
        // every container it no longer wants, so `always` revives nothing stale.
        let restart = match self.info().await {
            Ok(i) if i.name == "podman" => RestartPolicyNameEnum::ALWAYS,
            _ => RestartPolicyNameEnum::UNLESS_STOPPED,
        };
        // Pin an explicit loopback port below the kernel's ephemeral outbound
        // range (32768..60999) so container restarts, engine restarts, host
        // reboots, and running after Rig is uninstalled always keep the exact
        // same loopback port in HostConfig.PortBindings.
        let pinned_port = allocate_loopback_port()
            .map(|p| p.to_string())
            .unwrap_or_default();
        let host = HostConfig {
            network_mode: spec.network.clone(),
            binds: (!spec.volumes.is_empty()).then(|| {
                spec.volumes
                    .iter()
                    .map(|(v, path)| format!("{v}:{path}"))
                    .collect()
            }),
            // Loopback only: the proxy is the single public entry point.
            port_bindings: Some(HashMap::from([(
                port_key.clone(),
                Some(vec![PortBinding {
                    host_ip: Some("127.0.0.1".into()),
                    host_port: Some(pinned_port),
                }]),
            )])),
            restart_policy: Some(RestartPolicy {
                name: Some(restart),
                maximum_retry_count: None,
            }),
            memory: spec.memory_mb.map(|mb| i64::from(mb) * 1024 * 1024),
            nano_cpus: spec.cpu_millis.map(|m| i64::from(m) * 1_000_000),
            // Prevent a fork bomb inside an app/database container from exhausting host PIDs.
            pids_limit: Some(512),
            // Drop raw packet capability; apps/DBs do not need raw sockets.
            cap_drop: Some(vec!["NET_RAW".into()]),
            // Apps get no extra kernel privileges and cannot regain them.
            security_opt: Some(vec!["no-new-privileges".into()]),
            ..Default::default()
        };
        let mut body = ContainerCreateBody {
            image: Some(spec.image.clone()),
            env: Some(spec.env.iter().map(|(k, v)| format!("{k}={v}")).collect()),
            labels: Some(spec.labels.clone()),
            exposed_ports: Some(vec![port_key.clone()]),
            cmd: spec.command.clone(),
            networking_config: spec.network.as_ref().map(|network| NetworkingConfig {
                endpoints_config: Some(HashMap::from([(
                    network.clone(),
                    EndpointSettings {
                        aliases: Some(spec.aliases.clone()),
                        ..Default::default()
                    },
                )])),
            }),
            host_config: Some(host),
            ..Default::default()
        };
        let mut last_err = None;
        for attempt in 0..3 {
            if attempt > 0 {
                let fallback_port = if attempt == 1 {
                    allocate_loopback_port()
                        .map(|p| p.to_string())
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                if let Some(hc) = body.host_config.as_mut() {
                    hc.port_bindings = Some(HashMap::from([(
                        port_key.clone(),
                        Some(vec![PortBinding {
                            host_ip: Some("127.0.0.1".into()),
                            host_port: Some(fallback_port),
                        }]),
                    )]));
                }
            }
            let opts = CreateContainerOptionsBuilder::default()
                .name(&spec.name)
                .build();
            let created = self
                .api
                .create_container(Some(opts), body.clone())
                .await
                .with_context(|| format!("creating container {}", spec.name))?;
            let id = created.id;
            if let Err(e) = self.api.start_container(&id, None).await {
                let _ = self.remove(&id).await;
                last_err = Some(anyhow!(e).context(format!("starting container {}", spec.name)));
                continue;
            }
            let host_port = match self.host_port(&id, &port_key).await {
                Ok(p) => p,
                Err(e) => {
                    let _ = self.remove(&id).await;
                    return Err(e);
                }
            };
            return Ok(Started { id, host_port });
        }
        Err(last_err.unwrap_or_else(|| anyhow!("starting container {}", spec.name)))
    }

    async fn state(&self, id: &str) -> Result<ContainerState> {
        Ok(self
            .health(id)
            .await?
            .map_or(ContainerState::Missing, |h| h.state))
    }

    async fn health(&self, id: &str) -> Result<Option<Health>> {
        let c = match self.api.inspect_container(id, None).await {
            Ok(c) => c,
            Err(e) if is_not_found(&e) => return Ok(None),
            Err(e) => return Err(anyhow!(e).context("inspecting container")),
        };
        let host_port = first_published_port(&c);
        let restart_count = c.restart_count.unwrap_or(0);
        let Some(state) = c.state else {
            return Ok(Some(Health {
                state: ContainerState::Other("unknown".into()),
                restart_count,
                oom_killed: false,
                exit_code: None,
                host_port,
            }));
        };
        let current = match state.status {
            Some(ContainerStateStatusEnum::RUNNING) => ContainerState::Running,
            // A restarting container has already exited at least once; the
            // restart policy is only bringing it back.
            Some(
                ContainerStateStatusEnum::EXITED
                | ContainerStateStatusEnum::DEAD
                | ContainerStateStatusEnum::RESTARTING,
            ) => ContainerState::Exited(state.exit_code),
            Some(other) => ContainerState::Other(other.to_string()),
            None => ContainerState::Other("unknown".into()),
        };
        Ok(Some(Health {
            state: current,
            restart_count,
            oom_killed: state.oom_killed.unwrap_or(false),
            exit_code: state.exit_code,
            host_port,
        }))
    }

    async fn start(&self, id: &str) -> Result<()> {
        match self.api.start_container(id, None).await {
            Ok(()) => Ok(()),
            Err(e) if is_not_modified(&e) => Ok(()),
            Err(e) => Err(anyhow!(e).context("starting container")),
        }
    }

    async fn stop(&self, id: &str, grace_secs: u32) -> Result<()> {
        let opts = StopContainerOptionsBuilder::default()
            .t(i32::try_from(grace_secs).unwrap_or(10))
            .build();
        match self.api.stop_container(id, Some(opts)).await {
            Ok(()) => Ok(()),
            Err(e) if is_not_found(&e) || is_not_modified(&e) => Ok(()),
            Err(e) => Err(anyhow!(e).context("stopping container")),
        }
    }

    async fn remove(&self, id: &str) -> Result<()> {
        let opts = RemoveContainerOptionsBuilder::default()
            .force(true)
            .v(true)
            .build();
        match self.api.remove_container(id, Some(opts)).await {
            Ok(()) => Ok(()),
            Err(e) if is_not_found(&e) => Ok(()),
            Err(e) => Err(anyhow!(e).context("removing container")),
        }
    }

    fn logs_with(&self, id: &str, req: super::LogOptions) -> LogStream {
        let mut builder = LogsOptionsBuilder::default()
            .stdout(true)
            .stderr(true)
            .follow(req.follow)
            .timestamps(req.timestamps)
            .tail(&req.tail.to_string());
        if let Some(since) = req.since {
            builder = builder.since(i32::try_from(since).unwrap_or(i32::MAX));
        }
        let opts = builder.build();
        let want_ts = req.timestamps;
        // Following has no natural end, so it must not use the short timeout.
        let client = if req.follow { &self.long } else { &self.api };
        let stream = client.logs(id, Some(opts)).map(move |item| {
            let out = item.context("reading container logs")?;
            let stream = match out {
                LogOutput::StdErr { .. } => "stderr",
                _ => "stdout",
            };
            let raw = String::from_utf8_lossy(out.as_ref()).trim_end().to_string();
            let (ts, line) = if want_ts {
                split_log_timestamp(&raw)
            } else {
                (None, raw)
            };
            Ok(LogMsg { stream, line, ts })
        });
        Box::pin(stream)
    }

    async fn list_by_label(&self, label: &str, value: &str) -> Result<Vec<String>> {
        let filters = HashMap::from([("label".to_string(), vec![format!("{label}={value}")])]);
        let opts = ListContainersOptionsBuilder::default()
            .all(true)
            .filters(&filters)
            .build();
        let list = self
            .api
            .list_containers(Some(opts))
            .await
            .context("listing containers")?;
        Ok(list.into_iter().filter_map(|c| c.id).collect())
    }

    async fn remove_volume(&self, name: &str) -> Result<()> {
        match self
            .api
            .remove_volume(name, None::<RemoveVolumeOptions>)
            .await
        {
            Ok(()) => Ok(()),
            Err(e) if is_not_found(&e) => Ok(()),
            Err(e) => Err(anyhow!(e).context(format!("removing volume {name}"))),
        }
    }

    async fn remove_network(&self, name: &str) -> Result<()> {
        match self.api.remove_network(name).await {
            Ok(()) => Ok(()),
            // 403: containers are still attached, so it is still in use.
            Err(e) if is_not_found(&e) || is_status(&e, 403) => Ok(()),
            Err(e) => Err(anyhow!(e).context(format!("removing network {name}"))),
        }
    }

    async fn usage(&self, id: &str) -> Result<Option<Usage>> {
        // Not streaming and not one-shot: the engine waits one cycle and
        // fills in the previous CPU reading, which the percentage needs.
        let options = StatsOptionsBuilder::default()
            .stream(false)
            .one_shot(false)
            .build();
        let mut stats = self.api.stats(id, Some(options));
        match stats.next().await {
            Some(Ok(stats)) => Ok(usage_from(&stats)),
            Some(Err(e)) if is_not_found(&e) || is_status(&e, 409) => Ok(None),
            Some(Err(e)) => Err(anyhow!(e).context("reading container usage")),
            None => Ok(None),
        }
    }

    async fn shell(&self, id: &str, cols: u16, rows: u16) -> Result<Shell> {
        // Prefer bash when the image has it. The script is a constant: nothing
        // from the request reaches the command line.
        let script = "command -v bash >/dev/null 2>&1 && exec bash || exec sh";
        let exec = self
            .api
            .create_exec(
                id,
                CreateExecOptions {
                    attach_stdin: Some(true),
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    tty: Some(true),
                    env: Some(vec!["TERM=xterm-256color"]),
                    cmd: Some(vec!["/bin/sh", "-c", script]),
                    ..Default::default()
                },
            )
            .await
            .context("creating the shell")?;
        let started = self
            .long
            .start_exec(
                &exec.id,
                Some(StartExecOptions {
                    tty: true,
                    ..Default::default()
                }),
            )
            .await
            .context("starting the shell")?;
        let StartExecResults::Attached { output, input } = started else {
            bail!("the engine did not attach to the shell");
        };
        // The size can only be set once the process exists.
        self.resize_shell(&exec.id, cols, rows).await.ok();
        let output = output.map(|chunk| {
            chunk
                .map(LogOutput::into_bytes)
                .map_err(anyhow::Error::from)
        });
        Ok(Shell {
            id: exec.id,
            output: Box::pin(output),
            input,
        })
    }

    async fn exec(
        &self,
        id: &str,
        cmd: Vec<String>,
        env: Vec<String>,
        stdin: Option<ByteReader>,
        mut stdout: ByteWriter,
    ) -> Result<ExecOutcome> {
        use tokio::io::AsyncWriteExt;

        let exec = self
            .api
            .create_exec(
                id,
                CreateExecOptions {
                    attach_stdin: Some(stdin.is_some()),
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    tty: Some(false),
                    env: Some(env),
                    cmd: Some(cmd),
                    ..Default::default()
                },
            )
            .await
            .context("creating the command")?;
        let started = self
            .long
            .start_exec(&exec.id, None::<StartExecOptions>)
            .await
            .context("starting the command")?;
        let StartExecResults::Attached { mut output, input } = started else {
            bail!("the engine did not attach to the command");
        };

        // Feed stdin alongside reading stdout so neither side can stall the other.
        let feeder = stdin.map(|mut reader| {
            let mut input = input;
            tokio::spawn(async move {
                let copied = tokio::io::copy(&mut reader, &mut input).await;
                // Closing our half is how the command sees end of input.
                input.shutdown().await.ok();
                copied
            })
        });

        let mut stderr = Vec::new();
        while let Some(chunk) = output.next().await {
            match chunk.context("reading command output")? {
                LogOutput::StdOut { message } => stdout
                    .write_all(&message)
                    .await
                    .context("writing command output")?,
                LogOutput::StdErr { message } => {
                    stderr.extend_from_slice(&message);
                    if stderr.len() > STDERR_TAIL * 2 {
                        stderr.drain(..stderr.len() - STDERR_TAIL);
                    }
                }
                _ => {}
            }
        }
        stdout.flush().await.context("writing command output")?;
        stdout.shutdown().await.ok();
        let fed = match feeder {
            Some(feeder) => feeder
                .await
                .map_err(anyhow::Error::from)
                .and_then(|r| r.map(drop).map_err(anyhow::Error::from)),
            None => Ok(()),
        };

        let inspected = self
            .api
            .inspect_exec(&exec.id)
            .await
            .context("reading the command's exit code")?;
        // A command that fails early stops reading, which breaks the pipe; its
        // own exit code and stderr say more than the write error does.
        if inspected.exit_code == Some(0) {
            fed.context("feeding the command")?;
        }
        let start = stderr.len().saturating_sub(STDERR_TAIL);
        Ok(ExecOutcome {
            exit_code: inspected.exit_code,
            stderr: String::from_utf8_lossy(&stderr[start..]).trim().to_owned(),
        })
    }

    async fn resize_shell(&self, shell_id: &str, cols: u16, rows: u16) -> Result<()> {
        self.api
            .resize_exec(
                shell_id,
                ResizeExecOptions {
                    width: cols,
                    height: rows,
                },
            )
            .await
            .context("resizing the shell")
    }
}

impl DockerRuntime {
    async fn ensure_network(&self, name: &str) -> Result<()> {
        match self.api.inspect_network(name, None).await {
            Ok(_) => return Ok(()),
            Err(e) if is_not_found(&e) => {}
            Err(e) => return Err(anyhow!(e).context(format!("looking up network {name}"))),
        }
        let request = NetworkCreateRequest {
            name: name.to_string(),
            labels: Some(HashMap::from([(
                "rig.managed".to_string(),
                "true".to_string(),
            )])),
            ..Default::default()
        };
        match self.api.create_network(request).await {
            Ok(_) => Ok(()),
            // Another deployment created it between the two calls.
            Err(e) if is_status(&e, 409) => Ok(()),
            Err(e) => Err(anyhow!(e).context(format!("creating network {name}"))),
        }
    }

    /// The loopback port the engine picked for `port_key`.
    async fn host_port(&self, id: &str, port_key: &str) -> Result<u16> {
        // Podman can report the binding a moment after start returns.
        for _ in 0..20 {
            let c = self
                .api
                .inspect_container(id, None)
                .await
                .context("inspecting container")?;
            let port = c
                .network_settings
                .and_then(|n| n.ports)
                .and_then(|mut p| p.remove(port_key))
                .flatten()
                .and_then(|b| b.into_iter().find_map(|b| b.host_port))
                .and_then(|p| p.parse::<u16>().ok())
                .filter(|p| *p != 0);
            if let Some(port) = port {
                return Ok(port);
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        bail!("the engine did not publish port {port_key}; the container may have exited at once")
    }
}

/// Picks a free TCP port on `127.0.0.1` in `20000..=32000` (below the Linux
/// ephemeral outbound range `32768..60999`) so a pinned container port is not
/// stolen by random outbound host connections across restarts or reboots.
fn allocate_loopback_port() -> Option<u16> {
    const BASE: u16 = 20_000;
    const SPAN: u16 = 12_000;
    let start = rand::random::<u16>() % SPAN;
    for offset in 0..128 {
        let port = BASE + ((start + offset) % SPAN);
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Some(port);
        }
    }
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .ok()
        .and_then(|l| l.local_addr().ok())
        .map(|a| a.port())
        .filter(|p| *p != 0)
}

/// Extracts the first non-zero published host port from a container inspection.
fn first_published_port(c: &bollard::models::ContainerInspectResponse) -> Option<u16> {
    c.network_settings
        .as_ref()?
        .ports
        .as_ref()?
        .values()
        .flatten()
        .flatten()
        .find_map(|b| b.host_port.as_deref()?.parse::<u16>().ok())
        .filter(|p| *p != 0)
}

/// Turns one stats reading into CPU and memory use, the way `docker stats` does.
fn usage_from(stats: &ContainerStatsResponse) -> Option<Usage> {
    let cpu = stats.cpu_stats.as_ref()?;
    let memory = stats.memory_stats.as_ref()?;
    // A stopped container reports no memory use.
    let used = memory.usage?;
    let total =
        |c: &bollard::models::ContainerCpuStats| c.cpu_usage.as_ref().and_then(|u| u.total_usage);

    let cpu_percent = stats
        .precpu_stats
        .as_ref()
        .and_then(|before| {
            let spent = total(cpu)?.checked_sub(total(before)?)?;
            let elapsed = cpu
                .system_cpu_usage?
                .checked_sub(before.system_cpu_usage?)?;
            let cores = f64::from(cpu.online_cpus.filter(|n| *n > 0)?);
            (elapsed > 0).then(|| spent as f64 / elapsed as f64 * cores * 100.0)
        })
        .unwrap_or(0.0);

    // Page cache the kernel can drop at will is not the container's memory.
    let cache = memory
        .stats
        .as_ref()
        .and_then(|s| {
            s.get("inactive_file")
                .or_else(|| s.get("total_inactive_file"))
        })
        .copied()
        .unwrap_or(0);
    // An unlimited container reports the host's memory, or a huge sentinel.
    let limit = memory.limit.filter(|l| *l < (1 << 50)).unwrap_or(0);
    let (net_rx, net_tx) =
        stats
            .networks
            .iter()
            .flat_map(|n| n.values())
            .fold((0u64, 0u64), |(rx, tx), n| {
                (
                    rx.saturating_add(n.rx_bytes.unwrap_or(0)),
                    tx.saturating_add(n.tx_bytes.unwrap_or(0)),
                )
            });
    Some(Usage {
        cpu_percent,
        memory_bytes: used.saturating_sub(cache),
        memory_limit: limit,
        net_rx,
        net_tx,
    })
}

/// Tar a build context into an unnamed temp file.
///
/// Symlinks are stored as links, never followed: a repository must not be
/// able to pull files from outside its checkout into the build.
fn pack_context(dir: &Path) -> Result<std::fs::File> {
    use std::io::{Seek, SeekFrom};

    let file = tempfile::tempfile().context("creating temp file for build context")?;
    let mut tar = tar::Builder::new(file);
    tar.follow_symlinks(false);
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let name = entry.file_name();
        if name == ".git" {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            tar.append_dir_all(&name, &path)?;
        } else {
            tar.append_path_with_name(&path, &name)?;
        }
    }
    let mut file = tar.into_inner().context("finishing build context")?;
    file.seek(SeekFrom::Start(0))?;
    Ok(file)
}

/// Splits Docker/Podman's leading RFC 3339 timestamp (`2026-...Z message`) into
/// `(Some(ts), message)` when present.
fn split_log_timestamp(raw: &str) -> (Option<String>, String) {
    if let Some((head, rest)) = raw.split_once(' ')
        && chrono::DateTime::parse_from_rfc3339(head).is_ok()
    {
        return (Some(head.to_string()), rest.to_string());
    }
    (None, raw.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn context_skips_git_and_keeps_symlinks_as_links() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Dockerfile"), "FROM scratch\n").unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/config"), "secret").unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.txt"), "hi").unwrap();
        std::os::unix::fs::symlink("/etc/passwd", dir.path().join("leak")).unwrap();

        let mut file = pack_context(dir.path()).unwrap();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        let mut archive = tar::Archive::new(bytes.as_slice());
        let mut names = Vec::new();
        for entry in archive.entries().unwrap() {
            let entry = entry.unwrap();
            let name = entry.path().unwrap().to_string_lossy().into_owned();
            if name == "leak" {
                assert!(entry.header().entry_type().is_symlink());
                assert_eq!(entry.size(), 0, "symlink target content must not be packed");
            }
            names.push(name);
        }
        assert!(names.iter().any(|n| n == "Dockerfile"));
        assert!(names.iter().any(|n| n == "src/main.txt"));
        assert!(names.iter().any(|n| n == "leak"));
        assert!(!names.iter().any(|n| n.starts_with(".git")));
    }

    #[test]
    fn usage_matches_docker_stats_arithmetic() {
        let stats: ContainerStatsResponse = serde_json::from_value(serde_json::json!({
            "cpu_stats": { "cpu_usage": { "total_usage": 3_000_000_000u64 }, "system_cpu_usage": 40_000_000_000u64, "online_cpus": 4 },
            "precpu_stats": { "cpu_usage": { "total_usage": 2_000_000_000u64 }, "system_cpu_usage": 36_000_000_000u64, "online_cpus": 4 },
            "memory_stats": { "usage": 50_000_000u64, "limit": 268_435_456u64, "stats": { "inactive_file": 10_000_000u64 } },
        }))
        .unwrap();
        let usage = usage_from(&stats).unwrap();
        // 1s of CPU over 4s of host time on 4 cores is one full core.
        assert!((usage.cpu_percent - 100.0).abs() < 1e-9);
        assert_eq!(usage.memory_bytes, 40_000_000);
        assert_eq!(usage.memory_limit, 268_435_456);

        let stopped: ContainerStatsResponse =
            serde_json::from_value(serde_json::json!({ "cpu_stats": {}, "memory_stats": {} }))
                .unwrap();
        assert_eq!(usage_from(&stopped), None);
    }

    #[test]
    fn allocate_loopback_port_picks_dedicated_non_ephemeral_range() {
        let port = allocate_loopback_port().expect("should allocate a loopback port");
        assert!(
            (20_000..=32_000).contains(&port),
            "expected port in 20000..=32000, got {port}"
        );
    }
}

/// The fully qualified form of an image reference: `nginx:alpine` becomes
/// `docker.io/library/nginx:alpine`, `user/app` becomes `docker.io/user/app`.
/// References that already name a registry (`ghcr.io/…`, `localhost:5000/…`)
/// or an image id are returned unchanged.
pub fn qualify_image(image: &str) -> String {
    if image.starts_with("sha256:") {
        return image.to_owned();
    }
    match image.split_once('/') {
        Some((first, _)) if first.contains(['.', ':']) || first == "localhost" => image.to_owned(),
        Some(_) => format!("docker.io/{image}"),
        None => format!("docker.io/library/{image}"),
    }
}

#[cfg(test)]
mod qualify_tests {
    use super::{qualify_image, split_log_timestamp};

    #[test]
    fn parses_container_log_timestamps() {
        let (ts, msg) = split_log_timestamp("2026-11-22T10:00:00.123456Z hello world");
        assert_eq!(ts.as_deref(), Some("2026-11-22T10:00:00.123456Z"));
        assert_eq!(msg, "hello world");

        let (ts, msg) = split_log_timestamp("plain line without timestamp");
        assert!(ts.is_none());
        assert_eq!(msg, "plain line without timestamp");
    }

    #[test]
    fn short_names_get_the_docker_hub_registry() {
        assert_eq!(
            qualify_image("nginx:alpine"),
            "docker.io/library/nginx:alpine"
        );
        assert_eq!(
            qualify_image("gotify/server:3"),
            "docker.io/gotify/server:3"
        );
        assert_eq!(qualify_image("ghcr.io/org/app:1"), "ghcr.io/org/app:1");
        assert_eq!(qualify_image("localhost:5000/app"), "localhost:5000/app");
        assert_eq!(qualify_image("localhost/rig/app:x"), "localhost/rig/app:x");
        assert_eq!(
            qualify_image("docker.io/library/redis"),
            "docker.io/library/redis"
        );
        assert_eq!(qualify_image("sha256:abc"), "sha256:abc");
    }
}
