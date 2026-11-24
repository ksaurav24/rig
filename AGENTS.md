# Rig Monorepo — Agent & Developer Guide

## Workspace Layout

- `crates/rig-api` — Shared wire types and OpenAPI schemas (`utoipa`). Single source of truth for REST/SSE payloads.
- `crates/rig-client` — Shared typed HTTP, resumable SSE, WebSocket, and TLS-pinning client used by `rig-cli` (CLI + MCP) and `apps/desktop`.
- `crates/rig-server` — Rust + Axum + SQLite + Caddy control plane. Supports `--mode full|headless` and `--access public|lan|tailscale|local`.
- `crates/rig-cli` — `rig` CLI and `rig mcp` stdio MCP server, thin adapter over `rig-client`.
- `apps/desktop` — Tauri 2 native multi-server desktop app (`src-tauri/` Rust core + `src/` React/Tailwind/TanStack Virtual/xterm.js shell). See `apps/desktop/AGENTS.md`.
- `web` — Embedded single-server React dashboard compiled into `rig-server` for `--mode full`.

## Key Invariants

- **One client transport**: Auth headers, retries, SSE resume (`Last-Event-ID` / `?after=`), TLS certificate pinning (`lan` / `internal`), and version negotiation (`GET /api/v1/meta`) live exclusively in `crates/rig-client`.
- **Desktop security boundary**: `apps/desktop/src/` never calls `fetch()` or opens browser WebSockets against Rig servers and never handles bearer tokens. All networking runs in `apps/desktop/src-tauri/` with tokens in the OS keychain.
- **Local access mode**: `--access local` restricts API/dashboard requests to direct loopback peers (`127.0.0.1` / `::1`, ignoring forwarded headers) and omits the Rig dashboard route from Caddy while preserving app domains.
- **Device pairing**: Device tokens live in `api_tokens` with `kind = 'device'`, inheriting existing RBAC ceilings, audit logging, and `verify_still_valid` stream revocation.

## Verification Commands

```bash
# Default workspace: rig-api, rig-client, rig-server, rig-cli
cargo test

# Rig Desktop frontend typecheck + bundle
pnpm --dir apps/desktop build

# Rig Desktop native Rust core unit tests
cargo test -p rig-desktop

# Run Rig Desktop in dev mode (Vite HMR + Tauri window)
pnpm --dir apps/desktop tauri:dev

# Installer shell syntax check
bash -n dist/install.sh
```
