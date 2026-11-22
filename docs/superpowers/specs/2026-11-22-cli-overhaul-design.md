# Rig CLI Modularization, CLI↔MCP Deduplication, Feature Coverage & E2E Tests

Date: 2026-11-22
Status: Phase 1 Implemented (Phase 2 Planned)

---

## 1. Intent & Why

`rig-cli` is currently a 4-file prototype ([main.rs](file:///home/xlr8/code/rig/crates/rig-cli/src/main.rs) 690 LOC, [mcp.rs](file:///home/xlr8/code/rig/crates/rig-cli/src/mcp.rs) 738 LOC, [client.rs](file:///home/xlr8/code/rig/crates/rig-cli/src/client.rs) 215 LOC, [config.rs](file:///home/xlr8/code/rig/crates/rig-cli/src/config.rs) 189 LOC) that covers only 14 of ~85 server endpoints. Expanding it in its current shape creates three concrete problems:

1. **CLI ↔ MCP duplication and drift:** Both `main.rs` and `mcp.rs` reimplement app lookup, project filtering, deployment/rollback selection, and deploy log following. They have already drifted (CLI matches rollback ids with `ends_with`, while MCP does not; MCP URL-encodes rollback ids, while CLI does not).
2. **Operational sharp edges:** The token prompt echoes secrets to the terminal, dropped SSE connections fail active deploys even though the server supports `?after=` / `Last-Event-ID` resume, `Ctrl-C` during a deploy exits silently, `rig mcp` crashes immediately when unauthenticated and blocks all JSON-RPC requests during a 30-minute deploy, `--json` is ignored by several commands, and every error exits with code `1`.
3. **No end-to-end CLI tests:** Neither `cargo test` nor [scripts/e2e.sh](file:///home/xlr8/code/rig/scripts/e2e.sh) exercises the `rig` binary end-to-end.

The goal is a modular, DX-first `rig-cli` where the CLI and `rig mcp` share one tested operations layer, every core flow is verified end-to-end, and the codebase is ready to absorb full Rig API coverage cleanly in Phase 2.

---

## 2. Goals, Non-Goals & Anti-Goals

### Goals
- **Phase 1 — Foundation, CLI↔MCP Deduplication, Hardening & E2E Tests** *(CLI-only; no `rig-server` wire changes)*:
  - Modularize `rig-cli` into `lib.rs` + thin `main.rs` with `api/`, `ops/`, `commands/`, `mcp/`, `output.rs`, `error.rs`, and `config.rs`.
  - Deduplicate all shared behavior between the CLI and `rig mcp` into `ops/` and typed `api/` helpers.
  - Fix bugs/sharp edges B1–B9 (hidden token prompt, resumable SSE streams, `Ctrl-C` detach message, `rig mcp` unauthenticated startup + concurrent execution + cancellation + progress + `structuredContent` + untrusted log framing, strict ≥4-char ambiguity-checked deployment resolution, consistent `--json`/NDJSON, distinct exit codes, `env set` partial-failure reporting).
  - Ship quick perf & existing-endpoint wins: `tokio::try_join!` in `status`, `?project_id=` server-side filtering, gzip on JSON responses, `current_thread` Tokio runtime, optional `utoipa` behind a `schema` feature in `rig-api`, `rig deployments show/logs`, `rig apps show`, `rig completions <shell>`, and `--server` / `RIG_URL`+`RIG_TOKEN` stateless CI auth.
  - Add fast in-process E2E integration tests (`crates/rig-cli/tests/cli.rs`) spawning the compiled `rig` binary against a mock `axum` HTTP/SSE server, plus live CLI/MCP smoke checks in [scripts/e2e.sh](file:///home/xlr8/code/rig/scripts/e2e.sh).
- **Phase 2 — Full API Coverage, Incomplete Spec Features, Server Enablers & UX Polish** *(CLI + focused `rig-server` additions)*:
  - Add CLI commands and matching MCP tools for all remaining server endpoints: `rig exec` (WebSocket TTY + resize), `apps create/set/rm`, `domains ls/add/rm`, `db create/start/stop/rm/url/connect`, `backup ls/create/download/restore/rm`, `metrics`/`top`, `tokens ls/create/revoke` + `logout --revoke`, `projects rename/rm`, `members`, `audit`, `alerts`, `webhook`, `github repos`, plus MCP runtime container logs and `rollback_app --wait`.
  - Complete spec features from [RIG.md](file:///home/xlr8/code/rig/RIG.md) §14 backed by small `rig-server` enablers: `rig link` / `rig.toml` directory linking, `env import/pull/stdin/diff` + atomic bulk `PUT /api/v1/apps/{id}/env`, `logs --tail/--no-follow/--since/--timestamps/--grep` + server log query params, `GET /api/v1/apps/resolve?ref=`, deployment pagination (`?limit=&before=`), domains included in deploy SSE `end`, `start`/`restart`, and deploy cancel.
  - Polish daily UX: TTY/`NO_COLOR`-aware status colors, deploy spinner + phase/duration summary, relative timestamps, clickable `https://` URLs + `rig open`, table truncation/`--wide`/`-o`, destructive `--yes` confirmations, 409 attach-to-running-deploy prompt, `--wait-healthy`/`--timeout`, `-v`/`--debug` tracing, CLI/server version-skew warning, and `context` metadata/rename/current.

### Non-Goals (Deferred to a Follow-Up Spec)
- Browser OAuth / device-code sign-in and OS keychain (`keyring`) secret storage.
- Local-folder source upload and `rig deploy --local-build`.
- `rig update` CLI self-update mechanism.
- Cross-platform CLI release packaging (macOS/Windows/arm64 builds, switching `aws-lc-rs` → `ring`, Homebrew tap / standalone curl installer).
- Internal refactoring of `crates/rig-server` unrelated to Phase 2 API enablers.

### Anti-Goals (Failure Modes to Avoid)
- **Breaking backwards compatibility:** Existing CLI subcommands/flags and existing MCP tool names/schemas must keep working (only additive changes or explicit bug fixes).
- **Secret leakage:** Never echo tokens on terminal prompts, never add a secret-reveal MCP tool, and keep HTTP redirects disabled so Bearer tokens never leak cross-host.
- **Corrupted machine-readable streams:** `rig mcp` stdout must emit only newline-delimited JSON-RPC 2.0 frames; `--json` commands must emit valid JSON (or NDJSON for streaming `logs`) with zero prose on stdout.
- **Dependency bloat:** Keep `rig-cli` fast to compile and small (`opt-level = "z"`, `current_thread` Tokio, no `utoipa` in `rig-cli`).

---

## 3. Constraints

- **Workspace:** Rust 2024 edition (`Cargo.toml` workspace members: `rig-api`, `rig-server`, `rig-cli`).
- **Phase 1 boundary:** Zero changes to `rig-server` HTTP wire contracts. The only change outside `crates/rig-cli` in Phase 1 (besides `scripts/e2e.sh`) is gating `utoipa` in `crates/rig-api` behind a `schema` Cargo feature enabled by `rig-server`.
- **Exit code contract (CLI):**
  - `0` — success
  - `1` — generic / server / internal error
  - `2` — invalid CLI usage (handled by `clap`)
  - `3` — authentication / not signed in / expired token (`401` / `403` / missing context)
  - `4` — resource not found (`404` / unknown app or deployment)
  - `5` — network / server unreachable / timeout / stream broken after retries
  - `10` — deployment failed or cancelled
- **URL safety:** Every path parameter interpolated into a request URL must be encoded with `api::segment`.
- **Reuse existing crates:** Prefer crates already in `Cargo.lock` (`console`, `thiserror`, `chrono`, `axum` as a dev-dependency for mock E2E tests) plus lightweight `clap_complete` for shell completions.

---

## 4. Approaches Considered

1. **Single `rig-cli` crate (`lib.rs` + thin `main.rs`) with typed `api/`, shared `ops/`, `commands/`, and `mcp/` *(Chosen)*:**
   - Exposes `rig-cli` modules cleanly for unit and integration tests while shipping one `rig` binary.
   - Reuses `rig-api` types directly (without `utoipa`), with small typed endpoint methods (`api.app(id).deploy()`) and full control over SSE resume and Phase 2 WebSocket TTY.
2. **OpenAPI-generated Rust client from `rig-server openapi`:**
   - Eliminates hand-written path strings, but introduces a codegen pipeline step, duplicates or collides with `rig-api` wire types, and still requires hand-written SSE and WebSocket clients.
3. **Separate `crates/rig-client` workspace crate:**
   - Enforces a crate boundary, but adds a fourth workspace crate for a single consumer (`rig` already hosts both CLI and `rig mcp`). Can be split out later without changing the internal `api`/`ops` boundaries.

---

## 5. Detailed Design

### 5.1 Phase 1 — Modular Architecture (`crates/rig-cli`)

```
crates/rig-cli/
├── Cargo.toml
├── src/
│   ├── main.rs            # Thin entrypoint: clap parse, current_thread runtime, exit codes
│   ├── lib.rs             # Public exports for tests + top-level Cli & dispatch
│   ├── error.rs           # CliError enum + ExitCode mapping
│   ├── config.rs          # Config, Context, normalize_url, --server / RIG_URL + RIG_TOKEN
│   ├── output.rs          # Table printer, empty states, truncation, print_json, ndjson, bytes, label, short
│   ├── api/
│   │   ├── mod.rs         # Client, gzip JSON, segment(), typed resource paths (apps/deployments/env/etc.)
│   │   └── sse.rs         # SseParser (tracks event, id, data) + resumable stream helper (?after= / Last-Event-ID)
│   ├── ops/
│   │   ├── mod.rs         # Shared domain operations used by BOTH CLI and MCP
│   │   ├── apps.rs        # list_apps (?project_id=), resolve_app
│   │   ├── deployments.rs # list_deployments, get_deployment, resolve_deployment, deploy, rollback, follow, collect_logs
│   │   ├── env.rs         # list_env, parse_env_pairs, set_env_batch (partial-save reporting), reveal_env, delete_env
│   │   └── system.rs      # system_status (try_join!), list_projects, create_project, list_databases
│   ├── commands/
│   │   ├── mod.rs         # Command enum & dispatch
│   │   ├── auth.rs        # login (hidden prompt via console), logout
│   │   ├── context.rs     # context list / use
│   │   ├── status.rs      # status
│   │   ├── projects.rs    # projects list / create
│   │   ├── apps.rs        # apps list / show
│   │   ├── deploy.rs      # deploy / rollback / stop (+ Ctrl-C detach message)
│   │   ├── deployments.rs # deployments list / show / logs
│   │   ├── logs.rs        # logs (text + NDJSON under --json)
│   │   ├── env.rs         # env ls / set / get / rm
│   │   ├── databases.rs   # databases list
│   │   └── completions.rs # completions <shell>
│   └── mcp/
│       ├── mod.rs         # serve() entrypoint
│       ├── protocol.rs    # JSON-RPC 2.0 parsing, initialize, ping, notifications/cancelled, progress helpers
│       ├── tools.rs       # Tool registry, strict schema check, execution via ops/, structuredContent, untrusted logs
│       └── server.rs      # Concurrent task dispatcher, mpsc stdout writer, AbortHandle registry, lazy/unauth handling
└── tests/
    └── cli.rs             # Mock axum HTTP/SSE E2E test suite spawning the compiled `rig` binary
```

### 5.2 Phase 1 — Key Behavioral Specifications

1. **Optional `utoipa` in `rig-api`:**
   - `crates/rig-api/Cargo.toml`: `utoipa = { version = "6.0.0", features = ["chrono"], optional = true }`, `[features] schema = ["dep:utoipa"]`.
   - `crates/rig-api/src/lib.rs`: `#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]` on wire types.
   - `crates/rig-server/Cargo.toml`: `rig-api = { version = "0.1.0", path = "../rig-api", features = ["schema"] }`.
2. **Typed errors & exit codes (`error.rs`):**
   - `CliError` variants: `Auth(String)` → `3`, `NotFound(String)` → `4`, `Network(String)` → `5`, `DeployFailed(String)` → `10`, `Other(anyhow::Error)` → `1`.
   - HTTP status mapping in `api::Client`: `401` / `403` → `CliError::Auth`, `404` → `CliError::NotFound`, connect/timeout/stream retry exhaustion → `CliError::Network`.
   - Missing context / not signed in → `CliError::Auth`. Unknown app or deployment → `CliError::NotFound`.
3. **Auth & stateless CI override (`config.rs`):**
   - Global `--server <URL>` (`env = "RIG_URL"`) and `RIG_TOKEN` allow running any command without a saved config file: if `--server` / `RIG_URL` is provided alongside `RIG_TOKEN`, build an ephemeral `Context` (validated with `normalize_url`, honoring `--insecure` or `RIG_INSECURE=1` if needed).
   - `rig login` prompts for the token with echo disabled using `console::Term::stderr().read_secure_line()` (B1).
4. **Resumable SSE & `Ctrl-C` detach (`api/sse.rs`, `ops/deployments.rs`, `commands/deploy.rs`):**
   - `SseParser` parses `event:`, `id:`, and multi-line `data:` fields and emits `(event, id, data)`.
   - `Client::events_resumable(path, on_event)` tracks the last seen `id` (integer seq) and on mid-stream transport error reconnects up to 5 times with bounded backoff (`200ms`, `500ms`, `1s`, `2s`, `4s`), sending `Last-Event-ID: <id>` and `?after=<id>` until an `end` event arrives or retries exhaust (B2).
   - `rig deploy` and `rig rollback` race `follow_deployment` against `tokio::signal::ctrl_c()` (when following); on `Ctrl-C`, print to stderr:
     `Detached. Deployment <short> continues on the server. Follow it with: rig deployments logs <id>` and exit cleanly (`130`) (B3).
5. **Strict shared deployment resolution (`ops/deployments.rs`):**
   - Used identically by CLI `rollback` and MCP `rollback_app` (B6).
   - When `wanted` is `None`: first deployment with `status == Superseded && image_ref.is_some()`.
   - When `wanted` is `Some(q)`:
     - Exact match (`d.id == q`) always wins immediately.
     - Otherwise, require `q.len() >= 4` (if shorter, error asking for at least 4 characters).
     - Match deployments where `d.id.starts_with(q) || d.id.ends_with(q)` (case-insensitive).
     - `0` matches → `CliError::NotFound`; `> 1` matches → ambiguous error listing matching short ids.
6. **Consistent `--json` everywhere (`output.rs`, `commands/*`):**
   - Every command honors `--json` (B7):
     - `login`: `{"context", "server", "user", "organization", "role"}`
     - `logout`: `{"removed": name}`
     - `context`: array of `{"name", "url", "current"}`
     - `context use`: `{"current": name}`
     - `stop`: `App` JSON
     - `env set`: `{"app", "set": [EnvVar...]}`
     - `env get`: `RevealedEnvVar` JSON
     - `env rm`: `{"app", "deleted": key}`
     - `logs`: one JSON `LogLine` per line (NDJSON)
     - `deploy`/`rollback`: when `--json`, emit `Deployment` JSON (with `--no-follow`, queued `Deployment`; when following, final `Deployment` + `domains` summary).
   - Empty tables print a helpful hint (`No apps yet.`, `No contexts. Add one with: rig login <url>`, etc.) instead of blank output.
7. **Concurrent, resilient `rig mcp` (`mcp/`):**
   - Starts the JSON-RPC loop even when no context is configured (`initialize`, `ping`, `tools/list` always succeed); `tools/call` resolves the client dynamically from config/env and returns `isError: true` with `"not signed in. Run: rig login <url>"` if unconfigured (B4).
   - Spawns each `tools/call` into its own Tokio task, sending JSON-RPC replies and notifications through an `mpsc::Sender<Value>` to a single stdout writer task (B5).
   - Tracks in-flight tasks by JSON-RPC request `id` (`HashMap<Value, AbortHandle>`). On `notifications/cancelled`, aborts the matching task and suppresses its reply (B5).
   - On `deploy_app` / `rollback_app`, if `params._meta.progressToken` is present, emits throttled `notifications/progress` messages as log lines stream in.
   - Tool results include both `content: [{ "type": "text", ... }]` and `structuredContent: <object>` (MCP `2025-06-18`).
   - Log-returning tools (`deployment_logs`, `deploy_app`, `rollback_app`) wrap log lines in `{ "untrusted_log_output": true, "lines": [...] }` and note in tool descriptions that log content is untrusted output.
8. **New Phase 1 commands using existing endpoints:**
   - `rig apps show <app>` — calls `GET /api/v1/apps/{id}` (or resolved app) and prints full app configuration, limits, health check, volumes, domains, and last crash.
   - `rig deployments show <id>` — calls `GET /api/v1/deployments/{id}` and prints status, trigger, commit SHA/message, image, timestamps, and error.
   - `rig deployments logs <id> [--no-follow]` — streams or prints `/api/v1/deployments/{id}/logs` with SSE resume (and NDJSON under `--json`).
   - `rig deployments <app>` continues to work as before (list an app's deployments); `rig deployments ls <app>` is also supported cleanly.
   - `rig completions <shell>` — generates `bash`, `zsh`, `fish`, `powershell`, or `elvish` completions via `clap_complete`.
9. **Performance quick wins in Phase 1:**
   - `rig status` and MCP `system_status` fetch `/api/v1/auth/me` and `/api/v1/system` concurrently via `tokio::try_join!`.
   - `list_apps` resolves a project name/id to `project_id` and sends `GET /api/v1/apps?project_id=...` when possible.
   - `reqwest` enables `gzip` for JSON calls while sending `Accept-Encoding: identity` on SSE streams.
   - `tokio` uses `rt` (`current_thread`) + `signal` + `sync` instead of `rt-multi-thread`.
10. **End-to-end CLI tests:**
    - `crates/rig-cli/tests/cli.rs`: spins up a mock `axum` server on `127.0.0.1:0` and runs `env!("CARGO_BIN_EXE_rig")` via `std::process::Command` / `tokio::process::Command`, verifying:
      - `login` (with `RIG_TOKEN`), `context`, `context use`, `logout`, `--json` output for each
      - `--server` + `RIG_TOKEN` without any config file
      - `status` concurrent requests + JSON output
      - `apps` (with `--project`), `apps show`, ambiguity error (`project/app`)
      - `deploy` SSE streaming, mid-stream drop + `?after=` / `Last-Event-ID` resume to `running`, failed deploy exit code `10`
      - `deployments` list, `deployments show`, `deployments logs`
      - `rollback` default, exact id, ≥4-char prefix/suffix, <4-char rejection, ambiguous rejection
      - `env ls/set/get/rm`, `env set` partial-failure reporting, `--json`
      - `logs` text and `--json` NDJSON
      - Exit codes: `2` (usage), `3` (unauthenticated / 401), `4` (404 / missing app), `5` (unreachable server), `10` (failed deploy)
      - `completions` produces non-empty output
      - `rig mcp` stdio: unauthenticated startup (`initialize`/`tools/list` succeed, `tools/call` returns `isError`), concurrent `ping` + `tools/list` while `deploy_app` is blocked on SSE, `notifications/cancelled` aborts in-flight `deploy_app`, `notifications/progress` emitted when `progressToken` given, `structuredContent` + `untrusted_log_output` present.
    - `scripts/e2e.sh`: adds a `== cli & mcp` section using a created API token and temporary `RIG_CONFIG` to exercise `rig login`, `rig status`, `rig projects`, `rig apps`, `rig apps show`, `rig deployments`, `rig env set/ls/get/rm`, `rig mcp` (`initialize`, `tools/list`, `system_status`), and `rig logout` against the real `rig-server`.

---

### 5.3 Phase 2 — Full API Coverage, Incomplete Spec Features, Server Enablers & UX Polish

*(Implemented after Phase 1 is verified and merged/committed.)*

1. **Remaining existing-endpoint CLI & MCP commands:**
   - `rig exec <app>` (interactive raw-TTY WebSocket shell over `/api/v1/apps/{id}/terminal` with resize control frames).
   - `rig apps create|set|rm`, `rig domains ls|add|rm`.
   - `rig db ls|create|start|stop|rm|url|connect` and `rig backup ls|create|download|restore|rm`.
   - `rig metrics [app|db] [--range 1h|6h|24h]` and `rig top`.
   - `rig tokens ls|create|revoke` and `rig logout --revoke`.
   - `rig projects rename|rm`, `rig members`, `rig audit`, `rig alerts`, `rig webhook show|rotate`, `rig github repos`.
   - MCP tools: `app_logs` (runtime logs with untrusted framing), `get_metrics`, `list_domains` / `add_domain` / `remove_domain`, database lifecycle & backup tools, and `wait` flag on `rollback_app`.
2. **Incomplete spec features + focused `rig-server` enablers:**
   - `rig link <app>` & `rig.toml` repo config (auto-resolves `<app>` when omitted).
   - `rig env import <file>`, `rig env pull`, `rig env set KEY=-` (stdin), `rig env diff`, backed by an atomic bulk `PUT /api/v1/apps/{id}/env` endpoint on `rig-server`.
   - `rig logs --tail N --no-follow --since 10m --timestamps --grep <pat>`, backed by query params on `GET /api/v1/apps/{id}/logs`.
   - Server enablers: `GET /api/v1/apps/resolve?ref=`, deployment pagination (`?limit=&before=`), domains included in deploy SSE `end` event, `POST /api/v1/apps/{id}/start|restart`, and `POST /api/v1/deployments/{id}/cancel`.
3. **UX polish:**
   - Status colors respecting `NO_COLOR` and non-TTY detection; deploy spinner + elapsed timer + summary duration.
   - Relative timestamps (`3m ago`) in human tables (ISO kept in `--json`); clickable `https://` URLs + `rig open [app]`.
   - Table truncation to terminal width + `--wide` + `-o json|yaml|name`; `--yes` confirmations for `stop`/`rm`/`db rm`.
   - 409 deploy conflict prompt to attach to the in-progress deployment; `--wait-healthy` / `--timeout` for CI.
   - `-v`/`--debug` HTTP timing logs; CLI vs. `SystemInfo.version` skew warning; `rig context` user/org metadata, `rename`, and `current`.

---

## 6. Verification Plan

### Phase 1
- `cargo test -p rig-api -p rig-cli -p rig-server` — all unit and mock-HTTP/SSE E2E CLI + MCP tests pass.
- `cargo check --workspace --all-targets` and `cargo clippy --workspace --all-targets` — zero warnings/errors.
- Verify `cargo tree -p rig-cli -i utoipa` shows `rig-cli` no longer depends on `utoipa`.
- Verify release build size & startup speed (`cargo build --release -p rig-cli`).
- Run `scripts/e2e.sh` (or verify shell syntax `bash -n scripts/e2e.sh` if Docker/Podman isn't available in the test environment).

### Phase 2
- Unit + integration tests for every new server endpoint in `rig-server` and every new command/tool in `rig-cli/tests/cli.rs`.
- Full workspace build, clippy, and `scripts/e2e.sh` live run.

---

## 7. Left to the Builder

- Exact struct/module internal signatures inside `crates/rig-cli/src/{api,ops,commands,mcp}/`.
- Exact backoff jitter / progress throttling cadence in SSE resume and MCP `notifications/progress`.
- Table width defaults when stdout is not a TTY.
- Exact wording of empty-list hints and help `after_help` examples.
