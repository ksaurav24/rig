# Rig Monorepo — Agent & Developer Guide

## Workspace Layout

- `crates/rig-api` — Shared wire types and OpenAPI schemas (`utoipa`). Single source of truth for REST/SSE payloads.
- `crates/rig-client` — Shared typed HTTP, resumable SSE, WebSocket, and TLS-pinning client used by `rig-cli` (CLI + MCP) and `apps/desktop`.
- `crates/rig-server` — Rust + Axum + SQLite + Caddy control plane. Supports `--mode full|headless` and `--access public|lan|tailscale|local`.
- `crates/rig-cli` — `rig` CLI and `rig mcp` stdio MCP server, thin adapter over `rig-client`.
- `packages/ui` — Shared `@rig/ui` Fluid Functionalism component library, hooks, app primitives (`Logo`, `Page`, `Panel`, `Status`), and warm-graphite design tokens used by both `web` and `apps/desktop`.
- `apps/desktop` — Tauri 2 native multi-server desktop app (`src-tauri/` Rust core + `src/` React/Tailwind/TanStack Virtual/xterm.js shell). See `apps/desktop/AGENTS.md`.
- `web` — Embedded single-server React dashboard compiled into `rig-server` for `--mode full`.

## Key Invariants

- **One client transport**: Auth headers, retries, SSE resume (`Last-Event-ID` / `?after=`), TLS certificate pinning (`lan` / `internal`), and version negotiation (`GET /api/v1/meta`) live exclusively in `crates/rig-client`.
- **Desktop security boundary**: `apps/desktop/src/` never calls `fetch()` or opens browser WebSockets against Rig servers and never handles bearer tokens. All networking runs in `apps/desktop/src-tauri/` with tokens in the OS keychain.
- **Local access mode**: `--access local` restricts API/dashboard requests to direct loopback peers (`127.0.0.1` / `::1`, ignoring forwarded headers) and omits the Rig dashboard route from Caddy while preserving app domains.
- **Device pairing & interactive actions**: Device tokens live in `api_tokens` with `kind = 'device'`, inheriting existing RBAC ceilings, audit logging, and `verify_still_valid` stream revocation. Interactive user/account endpoints (password change, TOTP 2FA enrollment/disable/recovery codes, GitHub App connect/disconnect) use `auth.require_interactive(...)` so paired Desktop/mobile devices work alongside browser sessions while non-interactive `kind = 'api'` tokens remain blocked.
- **Portable server snapshots (`server_snapshot` capability)**: `POST /api/v1/system/snapshot/{export,inspect,restore}` stream zstd tarballs (optionally wrapped in the `RIGSNAP1` Argon2id + XChaCha20-Poly1305 envelope) without buffering large archives in browser memory. Export decrypts app/alert/GitHub secrets for cross-server portability; restore re-encrypts them under the target server's `master.key` while preserving target users, sessions, and paired devices.
- **CSP-safe media in Desktop**: Server-uploaded org logos and user avatars (`/api/v1/media/{id}`) are fetched through `rig-client` (`fetch_media_data_url` IPC) so strict Tauri CSP and pinned LAN TLS certificates are always honored.

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
