//! The container engine boundary.
//!
//! Everything Rig asks of Docker or Podman goes through [`Runtime`], so the
//! deploy pipeline never talks to an engine directly. Both engines are driven
//! over the Docker-compatible API by [`docker::DockerRuntime`].

pub mod docker;

use std::{collections::HashMap, path::PathBuf, pin::Pin};

use anyhow::Result;
use async_trait::async_trait;
use futures_util::Stream;
use tokio::sync::mpsc;

/// One line of build, deploy or container output.
#[derive(Debug, Clone)]
pub struct LogMsg {
    pub stream: &'static str,
    pub line: String,
}

/// Where a long-running engine operation reports progress.
///
/// Sending never fails the operation: if the receiver is gone the output is
/// dropped, since nobody is left to read it.
#[derive(Clone)]
pub struct LogSink(mpsc::Sender<LogMsg>);

impl LogSink {
    pub fn new(tx: mpsc::Sender<LogMsg>) -> Self {
        Self(tx)
    }

    pub async fn line(&self, stream: &'static str, text: impl AsRef<str>) {
        for line in text.as_ref().lines() {
            let line = line.trim_end();
            if line.is_empty() {
                continue;
            }
            let _ = self
                .0
                .send(LogMsg {
                    stream,
                    line: truncate(line, MAX_LINE),
                })
                .await;
        }
    }
}

const MAX_LINE: usize = 4096;

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

#[derive(Debug, Clone)]
pub struct EngineInfo {
    /// `docker` or `podman`.
    pub name: String,
    pub version: String,
}

pub struct BuildSpec {
    /// Directory sent to the engine as the build context.
    pub context: PathBuf,
    /// Dockerfile path relative to the context.
    pub dockerfile: String,
    pub tag: String,
    pub labels: HashMap<String, String>,
}

/// A build from a BuildKit frontend plan (Railpack) rather than a Dockerfile.
pub struct PlanSpec {
    /// Source directory, the build context.
    pub context: PathBuf,
    /// Directory holding `railpack-plan.json`.
    pub plan_dir: PathBuf,
    /// Scratch directory the build may write to.
    pub work_dir: PathBuf,
    /// BuildKit frontend image, pinned by digest.
    pub frontend: String,
    pub tag: String,
}

pub struct ContainerSpec {
    pub name: String,
    pub image: String,
    /// Port the process listens on inside the container. Published on a
    /// loopback port chosen by the engine.
    pub port: u16,
    pub env: Vec<(String, String)>,
    pub labels: HashMap<String, String>,
    pub memory_mb: Option<u32>,
    pub cpu_millis: Option<u32>,
    /// Replaces the image's default command.
    pub command: Option<Vec<String>>,
    /// Private network to join, created on demand, and the extra DNS names
    /// the container answers to on it.
    pub network: Option<String>,
    pub aliases: Vec<String>,
    /// Named volumes as `(volume, path inside the container)`.
    pub volumes: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct Started {
    pub id: String,
    pub host_port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerState {
    Running,
    /// Exited or dead, with the exit code when the engine reports one.
    Exited(Option<i64>),
    /// Created, restarting, paused: not serving, but not finished either.
    Other(String),
    Missing,
}

/// What the engine remembers about a container's past: enough to notice that
/// it died and came back between two looks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Health {
    pub state: ContainerState,
    /// Times the restart policy has brought the container back.
    pub restart_count: i64,
    /// The kernel killed the last run for using more memory than its limit.
    pub oom_killed: bool,
    pub exit_code: Option<i64>,
}

pub type LogStream = Pin<Box<dyn Stream<Item = Result<LogMsg>> + Send>>;

/// An interactive shell inside a container. Dropping it ends the session.
pub struct Shell {
    pub id: String,
    /// Raw terminal output, escape sequences included.
    pub output: Pin<Box<dyn Stream<Item = Result<bytes::Bytes>> + Send>>,
    pub input: Pin<Box<dyn tokio::io::AsyncWrite + Send>>,
}

/// A container's resource use at one moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Usage {
    /// Percent of one core.
    pub cpu_percent: f64,
    pub memory_bytes: u64,
    /// Zero when the container has no limit of its own.
    pub memory_limit: u64,
    /// Bytes received and sent since the container started, over all its networks.
    pub net_rx: u64,
    pub net_tx: u64,
}

/// How a non-interactive command inside a container ended.
#[derive(Debug, Clone)]
pub struct ExecOutcome {
    pub exit_code: Option<i64>,
    /// The last few KB of stderr, for error messages.
    pub stderr: String,
}

impl ExecOutcome {
    pub fn success(&self) -> bool {
        self.exit_code == Some(0)
    }
}

pub type ByteReader = Pin<Box<dyn tokio::io::AsyncRead + Send>>;
pub type ByteWriter = Pin<Box<dyn tokio::io::AsyncWrite + Send>>;

#[async_trait]
pub trait Runtime: Send + Sync {
    async fn info(&self) -> Result<EngineInfo>;

    /// Pull `image`, reporting layer progress to `log`.
    async fn pull(&self, image: &str, log: &LogSink) -> Result<()>;

    /// Build an image from a local context directory.
    async fn build(&self, spec: BuildSpec, log: &LogSink) -> Result<()>;

    /// Build `spec.tag` from a BuildKit frontend plan.
    async fn build_plan(&self, spec: PlanSpec, log: &LogSink) -> Result<()>;

    /// Create and start a container, returning its id and loopback port.
    async fn run(&self, spec: &ContainerSpec) -> Result<Started>;

    async fn state(&self, id: &str) -> Result<ContainerState>;

    /// The state plus restart history. `None` when the container is gone.
    async fn health(&self, id: &str) -> Result<Option<Health>>;

    /// Start a stopped container again.
    async fn start(&self, id: &str) -> Result<()>;

    /// Stop with a grace period. Succeeds if the container is already gone.
    async fn stop(&self, id: &str, grace_secs: u32) -> Result<()>;

    /// Force-remove. Succeeds if the container is already gone.
    async fn remove(&self, id: &str) -> Result<()>;

    /// Container output: the last `tail` lines, then live output if `follow`.
    fn logs(&self, id: &str, tail: u32, follow: bool) -> LogStream;

    /// Ids of every container carrying `label=value`, running or not.
    async fn list_by_label(&self, label: &str, value: &str) -> Result<Vec<String>>;

    /// Delete a named volume and the data in it. Succeeds if it is gone.
    async fn remove_volume(&self, name: &str) -> Result<()>;

    /// Delete a network. Succeeds if it is gone or still has containers.
    async fn remove_network(&self, name: &str) -> Result<()>;

    /// Start an interactive shell with a terminal of `cols` by `rows`.
    async fn shell(&self, id: &str, cols: u16, rows: u16) -> Result<Shell>;

    /// The content ID (`sha256:…`) of a local image, or `None` when the
    /// engine no longer has it.
    async fn image_id(&self, image: &str) -> Result<Option<String>>;

    /// Current resource use of a container. `None` when it is not running.
    async fn usage(&self, id: &str) -> Result<Option<Usage>>;

    /// Tell a running shell that its terminal changed size.
    async fn resize_shell(&self, shell_id: &str, cols: u16, rows: u16) -> Result<()>;

    /// Run `cmd` in a running container without a TTY, streaming its stdout
    /// into `stdout` and feeding it `stdin` when given. `env` holds
    /// `KEY=value` pairs, which keeps secrets off the command line.
    async fn exec(
        &self,
        id: &str,
        cmd: Vec<String>,
        env: Vec<String>,
        stdin: Option<ByteReader>,
        stdout: ByteWriter,
    ) -> Result<ExecOutcome>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_respects_char_boundaries() {
        assert_eq!(truncate("abc", 10), "abc");
        assert_eq!(truncate("héllo", 2), "h…");
        assert_eq!(truncate("héllo", 3), "hé…");
    }

    #[tokio::test]
    async fn sink_splits_and_drops_blank_lines() {
        let (tx, mut rx) = mpsc::channel(8);
        LogSink::new(tx).line("build", "one\n\n  two  \r\n").await;
        assert_eq!(rx.recv().await.unwrap().line, "one");
        assert_eq!(rx.recv().await.unwrap().line, "  two");
        assert!(rx.try_recv().is_err());
    }
}
