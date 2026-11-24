# Rig Desktop: Multi-Server Control App, Headless Mode & Local Exposure

Date: 2026-11-24
Status: Concept, finalized in discussion and not yet implemented

---

## 1. Intent & Why

Today every Rig server has its own embedded dashboard, CLI access and MCP server. Someone who runs two or more servers ends up with two or more dashboards, logins and browser tabs. Every server also keeps its dashboard running whether or not anyone is using it.

**Goal:** a native desktop app (Linux, macOS, Windows, with mobile later) that can do everything for many Rig servers from one place:

- projects, apps, deployments and rollbacks
- env vars, Git and GitHub, databases and backups
- logs, metrics, alerts, the terminal
- members, tokens and settings

Alongside it:

- **Headless mode**, where a server runs the API only and serves no dashboard.
- A new **`local` exposure mode**, where the API isn't reachable from the network at all.

## 2. Decisions (finalized)

| # | Topic | Decision |
|---|---|---|
| D1 | What headless means | `--mode headless`: `rig-server` serves the API only. No dashboard assets, no SPA routes. Already in the installer as "coming soon", and now gets built. |
| D2 | Where UI and exposure are set | Both stay installer and wizard choices (`--mode`, `--access`). No new server config keys. |
| D3 | Exposure modes | Existing `--access public\|lan\|tailscale`, plus a new `local` mode (loopback only). |
| D4 | SSH tunnel transport | **A later phase.** v1 supports `public`, `lan` and `tailscale` servers. `local` servers can be managed from the CLI/MCP on the box until the SSH transport ships. |
| D5 | Multi-server UX | One server at a time, chosen with a switcher. **No combined views** across servers. |
| D6 | Network types | HTTPS with a domain, LAN (self-signed internal CA) and Tailscale all supported in v1. |
| D7 | Platforms | Desktop first (Linux, macOS, Windows). Mobile later, on the same stack. |
| D8 | Tech stack | **Tauri 2 with a Rust core and a React UI**, not GPUI. See §4. |
| D9 | Auth | **Device pairing** with revocable, named device tokens, kept **alongside** the existing password, TOTP and session login, not replacing it. |
| D10 | Features only the desktop app has | **All in v1:** deploy from a local Git checkout, drag-and-drop `.env` import, and a terminal into the container. |
| D11 | Repo layout | Monorepo. The desktop app, shared UI and shared client crate live in this repo and change together. |

## 3. Server side

### 3.1 Install modes (`--mode`)

| Mode | Dashboard | API | Notes |
|---|---|---|---|
| `full` (default) | served (embedded `web/dist`, pre-compressed by `crates/rig-server/build.rs`) | yes | Today's behavior. |
| `headless` | **not served** | yes | Dashboard routes return 404. Setup and login are done from the CLI or desktop app. |

Implementation notes:
- Headless is a runtime switch: the same binary, with the static/SPA router left out. This keeps one release artifact.
- An optional cargo feature could leave the ~2.3 MB of assets out of the binary for minimal builds. Nice to have, not required.
- The wizard (`crates/rig-server/src/wizard.rs`, `ModeChoice::Headless`) and `dist/install.sh` (line ~132, currently `die "headless mode is coming soon"`) drop the "coming soon" block.
- First-run setup (`/api/v1/setup`) must be possible without the browser: `rig setup` from the CLI, or the "Add server → Initialize" flow in the desktop app.
- A headless server can always be switched to full later, and back, without reinstalling.

### 3.2 Exposure modes (`--access`)

The existing logic lives in `crates/rig-server/src/access.rs`. The same IP ranges drive both Caddy's `remote_ip` matcher and Rig's own access check.

| Access | Who can reach the API and dashboard | How the desktop app connects |
|---|---|---|
| `public` | anyone (through Caddy on the domain) | HTTPS with a public certificate (`--tls auto`) |
| `lan` | private ranges (`LAN_RANGES`) | HTTPS with the internal CA (`--tls internal`), pinned during pairing |
| `tailscale` | the tailnet (CGNAT `100.64/10`, plus the Tailscale cert) | HTTPS through MagicDNS, already encrypted by WireGuard |
| **`local` (new)** | **loopback only** (`127.0.0.1` / `::1`) | **SSH tunnel (later phase)**. Until then, the CLI/MCP on the box. |

What `local` means:
- Rig answers only on loopback. Caddy publishes no route for the Rig API or dashboard domain. App domains are **not** affected and still follow their own per-domain access.
- Add it to the `Access` enum in `rig_api`, to the `--access` parser and help text in `install.sh`, and to `prompt_access` in the wizard, with the label "Local only (SSH / on-box)".
- Peer check: only loopback peers are allowed. Forwarded headers are ignored, consistent with the existing direct-peer rule.
- The wizard warns: "The desktop app can reach this server only over an SSH tunnel (coming later) or from this machine."

Every mode and access combination is valid. For example, `headless + public` is an API-only server on a domain, and `full + local` is a dashboard that opens only through SSH port-forwarding or on the box itself.

## 4. Tech stack: Tauri 2

### 4.1 Why Tauri over GPUI

| | Tauri 2 + React | GPUI |
|---|---|---|
| Reuse the current React dashboard | yes, most components | no, a full rewrite |
| Mobile later | iOS and Android supported in Tauri 2 | no |
| Maintained by one developer plus agents | React/TS and Rust: the best-known stacks for agents | small ecosystem, unstable API |
| Ready-made pieces | xterm.js, uPlot, TanStack Virtual, existing UI kit | you build them yourself |
| Raw rendering performance | good, with the rules in §7 | best |

GPUI wins only on raw rendering speed. The performance rules in §7 close most of that gap.

### 4.2 WASM: not used for the UI

The webview's weight comes from the browser engine (its processes, layout and painting), not from how fast JavaScript runs. WASM UI frameworks (Leptos, Yew, Dioxus-web) render into the same webview and add a bridge between JavaScript and WASM. CPU-heavy work goes into the **native Rust core**, which is faster than WASM. Dioxus Blitz (a native renderer) is worth watching but is too experimental to build on.

### 4.3 Repo layout

```
crates/
  rig-api/        # shared wire types (exists)
  rig-client/     # NEW: typed HTTP+SSE+WS client extracted from crates/rig-cli/src/api/
  rig-cli/        # uses rig-client (CLI + MCP)
  rig-server/
apps/
  desktop/        # NEW: Tauri 2 app
    src-tauri/    # Rust core: server registry, connections, keychain, streams
    src/          # desktop shell (switcher, windows, tray) around packages/ui
packages/
  ui/             # NEW: shared React screens/components used by web/ and apps/desktop
web/              # embedded dashboard (the full mode), consumes packages/ui
```

- `rig-client` is the **single place** for auth headers, retries, SSE resume (`Last-Event-ID` / `?after=`, already supported by the CLI's `api/sse.rs`), version negotiation and, later, the SSH transport. The CLI, MCP and desktop app all use it.
- `packages/ui` takes a data-access adapter, so the same screen works in both places:
  - in the browser, `fetch` with a session cookie
  - in the desktop app, Tauri `invoke` and `Channel`

## 5. Communication

### 5.1 Networking lives in the Rust core, not the webview

The UI never makes HTTP requests. It calls Tauri commands, and the Rust core makes the requests with `rig-client`. Benefits:
- no CORS or `check_origin` issues (`crates/rig-server/src/auth.rs:351`)
- pinned certificates for `lan` and `internal` TLS
- tokens stay in the OS keychain and never reach JavaScript
- the SSH transport can be added later without changing the UI
- no browser limit of six connections per host

### 5.2 Wire protocol

| Traffic | Transport | Notes |
|---|---|---|
| Commands and queries | REST/JSON, `/api/v1/*` | Existing utoipa-documented API. Desktop types come from `rig-api` / OpenAPI. |
| Per-resource streams (deploy logs, app logs, metrics) | **SSE** (existing) | Already resumable. The Rust core reconnects with `Last-Event-ID`. |
| Server event feed (status changes, deploy started/finished/failed, crashes from `watch.rs`, alerts) | **NEW: one SSE stream per server**, `GET /api/v1/events` | Light and resumable. Drives the sidebar health dots and notifications for every connected server. |
| Interactive sessions (container terminal, exec) | **WebSocket** (existing `routes/terminal.rs`) | Opened only while the terminal is visible. |

Note: an earlier idea was one multiplexed WebSocket. **SSE was chosen instead** because the server and CLI already have SSE with resume, it works through any proxy, and long-lived streams already re-check their credentials through `AuthSnapshot` / `verify_still_valid`.

### 5.3 Version and capability negotiation

`GET /api/v1/meta` (new, no auth required, minimal) returns `{ server_version, api_version, mode: full|headless, access, capabilities: [...] }`. The desktop app:
- hides or disables features an older server doesn't support instead of failing
- shows "update available" for servers running an older version
- refuses to connect, with a clear message, below a minimum `api_version`

### 5.4 Connection lifecycle per server

| Server state | What the app keeps open |
|---|---|
| **Active** (selected) | event SSE + the SSE streams for visible views + a terminal WS if open |
| **Inactive** (paired, not selected) | event SSE only (heartbeat + notifications) |
| **Offline** | exponential backoff, a grey dot, cached data shown read-only |

Switching servers closes the old server's view streams and keeps its cache (stale-while-revalidate), so switching back is instant.

## 6. Auth: device pairing, alongside the existing login

The existing pieces stay as they are:
- password login with TOTP and recovery codes
- sessions (`create_session`, `rotate_sessions`)
- API tokens (`/api/v1/tokens`, `Via::ApiToken`)
- roles `Viewer < Developer < Admin < Owner`, scoped to an org
- audit log

Device pairing is built **on top of** API tokens. It is a new token kind, not a separate system.

### 6.1 Pairing flow

1. **The server creates a pairing code:**
   - CLI on the box: `rig devices pair`
   - or the dashboard (full mode): Settings → Devices → Pair
   It shows a short one-time code (about 8 characters, valid 5 min, single use, rate-limited through `check_rate`), a QR code, and the server's **TLS certificate fingerprint**.
2. **In the app:** Add server → enter the URL (or scan the QR, on mobile later) → enter the code → for `lan`/`internal` TLS, confirm the fingerprint (trust on first use, then pinned).
3. **The server issues a device token:**
   - tied to the user and org that created the pairing code, with role ≤ that user's role
   - named after the device ("xlr8-thinkpad · Rig Desktop")
   - records last-used time and IP
   - audited
4. **The app stores the token in the OS keychain:** Keychain, Credential Manager, or Secret Service/libsecret.

**Other way in:** "Sign in with password", which runs the normal login with TOTP and then creates a device token automatically. Useful when you have no shell on the box.

### 6.2 Device management

- `rig devices list|revoke <id>`, plus the Devices page in the dashboard and in the app.
- Revoking cuts open streams within the existing `verify_still_valid` re-check interval.
- Second-factor changes still need a real browser session, as today (`routes/account.rs:153`).

### 6.3 Later hardening (not in v1)

- tokens that rotate automatically (refresh on use, expire after N idle days)
- an ed25519 keypair per device that signs requests, so a token leaked on its own is useless
- `local` with SSH: the SSH key alone acts as the login (the tunnel peer is loopback, and a device token is still required)

## 7. Performance rules

- **The Rust core does the heavy work:**
  - log ring buffers per stream
  - ANSI parsing
  - search and filter
  - metrics downsampling
  The UI gets only visible data, in batches through Tauri `Channel`s, at most one batch per animation frame.
- **Render less:**
  - TanStack Virtual for logs and tables
  - xterm.js with the WebGL renderer
  - uPlot (canvas) for charts
- **Only the active server streams data** (see §5.4).
- **Linux/WebKitGTK:** the weakest engine, so fewer animations and compositing workarounds there.
- **Budgets checked in CI:**
  - cold start < 1 s
  - idle RAM < 150 MB
  - smooth log tail at 5k lines/s
  - server switch < 100 ms

## 8. Desktop UX

- **Server switcher** down the left side, like Slack workspaces: avatar or initials, a health dot, ⌘/Ctrl+1…9, and drag to reorder.
- **Inside a server:** the same information architecture as the dashboard (Projects → Apps → Deployments / Env / Domains / Logs / Metrics / Terminal; Databases; Backups; Alerts; Members; Settings).
- **Command palette** (⌘K) that works within the current server.
- **Native integrations:**
  - OS notifications (deploy failed or finished, app crashed, alerts) from **every** paired server, with a click that jumps to that server
  - tray or menu-bar status
  - deep links like `rig://server/<id>/app/<app>`
  - auto-update through the Tauri updater

## 9. Features only the desktop app has (all v1)

1. **Deploy from a local Git checkout.** Choose a local repo, and the app shows its branch and remote and maps it to a Rig app. "Deploy this commit" pushes or creates a deploy through the existing deploy API (`crates/rig-server/src/deploy.rs`). It warns about uncommitted or unpushed changes.
2. **Drag-and-drop `.env`.** Drop a file onto an app's Env tab, and the Rust core parses it locally. The app shows a diff against the current values: added, changed, unchanged. Values that are already secret are masked, since secret values are never returned by the server. Applying sends one batch request.
3. **Container terminal.** xterm.js connected to the existing terminal WebSocket, with tabs, copy and paste, and resize. Opened only on demand.

## 10. Phasing

| Phase | Scope |
|---|---|
| **P0: test** | Throwaway Tauri app that wraps the current dashboard and streams a heavy fake log through the Rust core on Linux/WebKitGTK. Measure it against the §7 budgets before committing. |
| **P1: server enablers** | Headless mode; the `local` access mode; `GET /api/v1/meta`; `GET /api/v1/events` SSE; device token kind + pairing endpoints + `rig devices`; headless-friendly `rig setup`. |
| **P2: shared foundations** | Extract `crates/rig-client`; extract `packages/ui` with a data adapter; `web/` uses it unchanged. |
| **P3: Desktop v1** | Server registry, pairing, keychain, switcher, every dashboard feature, notifications, tray, deep links, auto-update, plus the three desktop-only features (§9). Linux, macOS and Windows builds in CI. |
| **P4: SSH transport** | `local` servers reachable through an SSH tunnel in `rig-client`, using the existing SSH config and agent. |
| **P5: mobile** | Tauri 2 iOS/Android on the same `packages/ui` and `rig-client`, with QR pairing. |

## 11. Open items

- Code signing and notarization: Apple Developer ID and a Windows signing cert, which affect release CI.
- Whether `/api/v1/meta` should hide `access` and `mode` from unauthenticated callers. Proposed: return only `server_version` and `api_version` without auth.
- The exact rate limits and lockout for pairing codes.
- How long cached data for offline servers is kept, and whether it is encrypted at rest.
