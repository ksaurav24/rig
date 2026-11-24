# Rig Desktop (`apps/desktop`) — Agent & Developer Guide

Rig Desktop is a Tauri 2 native multi-server control app for Rig (Linux, macOS, Windows), backed by `crates/rig-client` and `crates/rig-api`.

## Architectural Rules (Do Not Violate)

1. **Zero HTTP in the Webview**: `src/` NEVER calls `fetch()` or opens browser `WebSocket`s directly against Rig servers, and NEVER touches bearer tokens. All network calls go through `src/lib/ipc.ts` -> `src-tauri/src/commands.rs` -> `crates/rig-client`.
2. **Tokens Stay in the OS Keychain**: Device tokens live in `src-tauri/src/keychain.rs` (Keychain / Credential Manager / Secret Service). `servers.json` (`src-tauri/src/registry.rs`) stores only non-secret metadata, TLS pins, and cached read-only snapshots.
3. **Heavy Work Stays in Native Rust**:
   - Log ring buffers (50k lines/stream), zero-regex ANSI SGR parsing, and search live in `src-tauri/src/log_engine.rs`.
   - Frames are pushed to the UI via Tauri `Channel` at most once per animation frame (`16ms` / 60Hz).
   - `.env` parsing & masked secret diffing live in `src-tauri/src/env_diff.rs`.
   - Local Git inspection & zstd deduplicated source uploads live in `src-tauri/src/local_git.rs`.
   - Container terminal WebSockets live in `src-tauri/src/terminal.rs`.
4. **Connection Lifecycle**:
   - Every paired server maintains one lightweight `GET /api/v1/events` SSE loop (`src-tauri/src/connection.rs`) driving left-rail health dots and OS notifications.
   - Switching servers immediately cancels view-level streams (log SSEs + terminals) and switches via cached snapshots for `< 100ms` latency.
5. **WebKitGTK Discipline**: Keep CSS animations minimal and avoid `backdrop-filter: blur(...)` on scrolling log/table panes. Use `@tanstack/react-virtual` for logs and long lists.

## Commands

```bash
# Typecheck + build frontend bundle
pnpm --dir apps/desktop build

# Run Rust desktop unit tests (registry, ANSI parser, ring buffer, .env diff)
cargo test -p rig-desktop

# Launch desktop app in dev mode (Vite HMR + native Tauri window)
pnpm --dir apps/desktop tauri:dev

# Standalone release binary (embeds ../dist via custom-protocol)
pnpm --dir apps/desktop build && cargo build --release -p rig-desktop --features custom-protocol

# Run server + client + CLI workspace tests
cargo test
```

## Performance Budgets (§7 of spec)

- Cold start: `< 1 s`
- Idle RAM: `< 150 MB`
- Log tail throughput: `>= 5,000 lines/s` smooth (test via the built-in **Perf Benchmark** view or `start_log_benchmark` IPC command)
- Server switch: `< 100 ms` (measured live in Settings → Desktop Performance Diagnostics)
