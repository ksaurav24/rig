# Rig

A self-hosted platform for deploying apps and databases on your own server.
It's one Rust binary with an embedded dashboard, a CLI (`rig`) and an MCP
server, with Caddy in front for domains and HTTPS. The full spec is in
[RIG.md](RIG.md).

## Put it on a server

```sh
curl -fsSL https://github.com/ksaurav24/rig/releases/latest/download/install.sh | sudo bash
```

Or pass flags directly (e.g. `--podman`, `--docker`, `--domain rig.example.com`, `--tls auto|tailscale|internal|off`, `--access public|lan|tailscale`, `--yes`), or build a release bundle from source:

```sh
scripts/package.sh                      # -> dist/rig-<version>-x86_64-linux.tar.gz
scp dist/rig-*-x86_64-linux.tar.gz you@vm:
ssh you@vm 'tar xzf rig-*.tar.gz && cd rig-*-linux && sudo ./install.sh'
```

Then open the printed HTTPS address and enter the one-time setup token. The full walkthrough
(CLI flags, HTTPS, password reset, first deploy, workspace branding, backups, upgrades) is in
[docs/deploy-on-a-server.md](docs/deploy-on-a-server.md).

## Local deploys, container adoption & portable server snapshots

- **Deploy without Git (`rig up`):** Deploy a local project directory or a pre-compiled Linux ELF binary straight from your laptop with zero Git push required. Respects `.gitignore` and `.dockerignore`, deduplicates unchanged files and OCI layers via a content-addressed blob cache, streams `zstd`-compressed deltas over Rig's API, wraps static/dynamic binaries automatically into `scratch`/`distroless` OCI images, and performs a health-checked zero-downtime swap.
- **One-click adopt (`rig adopt`):** Discover running Docker/Podman containers, Docker Compose stacks, or Coolify/Dokploy workloads, preview the generated `rig.toml` and Podman Quadlet (`.container` / `.network`) units before touching anything, encrypt detected secrets into Rig's vault, preserve exact host bind mounts and volumes in-place, and hand off traffic with automatic rollback on health-check failure.
- **Full portable server snapshots (`rig snapshot` & MCP):** Download a single portable `zstd` snapshot archive of an entire Rig server (organization branding & media, projects, apps, decrypted-for-portability env secrets, domains, volumes, alerts, live database dumps, and optional GitHub App) with optional Argon2id + XChaCha20-Poly1305 passphrase encryption. Inspect and restore snapshots onto **any** Rig server (`merge` or `replace` mode) — all secrets are automatically re-encrypted under the target server's `master.key` while preserving target users, sessions, and paired devices. Exposed via CLI (`rig snapshot download|inspect|restore`), Rig Desktop, and MCP (`export_server_snapshot`, `inspect_server_snapshot`, `restore_server_snapshot`).

## Rig Desktop (`apps/desktop`)

A native Tauri 2 multi-server desktop application (Linux, macOS, Windows) backed by `rig-client` with OS keychain token storage, pinned TLS certificates for LAN servers, zero direct HTTP in the webview, and native Rust log/terminal streaming:

- **Multi-server switcher & live health rail:** Switch between paired servers in `< 100 ms` via cached read-only snapshots, live SSE event feeds, and native OS notifications.
- **App Catalog & full app lifecycle:** One-click deployment for 17 self-hosted Docker templates (auto-generated secrets & persistent volumes), GitHub App connect + repo/branch deploy, local folder deploys, `.env` secret diffing, interactive xterm.js container shell, deploy webhook reveal/rotate, CPU core limits (`cpu_millis`), persistent volumes, and project edit/delete dialogs (with safe database migration).
- **Historical metrics charts (`1h` / `6h` / `24h`):** Lightweight SVG area charts for CPU, memory, and network throughput across the host, individual apps, and managed databases.
- **Managed databases & local backup downloads:** Provision PostgreSQL, MySQL, MariaDB, Redis, and MongoDB, inspect usage metrics, and stream database backups directly to local disk via native save dialogs.
- **Server Snapshot Backup, Periodic Reminders & Cross-Server Restore (`Settings → Backup & Restore`):** Download encrypted or plaintext full-server snapshots, configure per-server periodic backup reminders (daily/weekly/biweekly/monthly/custom with 24h snooze, overview banner, and native OS notifications), dry-run inspect archives with conflict detection, and restore onto any paired Rig server.
- **Account, 2FA & Organization Branding (`Settings → Account & 2FA`, `General & Network`):** Manage profile display name and avatar (12 presets, upload, URL, reset), change password, enroll/disable TOTP 2FA with QR codes and downloadable recovery codes, upload/reset organization logos, and configure environment badge colors.

## Dashboard & customization

- **Organization branding (`Settings → General`):** Set your organization name, upload a custom logo (PNG, JPEG, WebP, or GIF) or paste an HTTPS image link, choose a workspace brand accent color, and optionally add an environment badge (e.g. `Production`, `Staging`, `Homelab`). Branding automatically applies to the sidebar, browser tab title, favicon, and split-screen sign-in page.
- **Account profiles (`Account`):** Edit your display name and pick an avatar from 12 built-in presets, upload a photo (auto-cropped and resized), or paste an image URL.
- **Personal workspace preferences (`Account`):** Customize theme (`Light` / `Dark` / `System`), personal accent color override, UI density (`Default` / `Compact`), corner style (`Rounded` / `Sharp` / `Pill`), sidebar layout (`Classic` / `Floating` / `Inset`) with persisted collapse and hover-peek, pinned projects in the sidebar, default home page, timestamp format (relative vs. exact, local vs. UTC, 12h/24h), and log/terminal font size and line wrapping.

## Develop

| Command | What it does |
| --- | --- |
| `scripts/dev.sh` | Rig, a Caddy container and the Vite dev server, with hot reload (`ENGINE=podman` uses Podman) |
| `scripts/e2e.sh` | End-to-end API checks against local Docker (`OFFLINE=1` skips the network, `ENGINE=podman` uses Podman) |
| `cd web && pnpm test:ui` | Browser flows with Playwright against a built dashboard |
| `scripts/vm-test.sh` | Installs the release bundle on a throwaway systemd "VM" and deploys an app (`VM_ENGINE=podman` tests Podman) |
| `cd web && pnpm gen:api` | Regenerates the TypeScript API types from the server's OpenAPI document |
| `pnpm --dir apps/desktop tauri:dev` | Run Rig Desktop in dev mode (Vite HMR + native Tauri window) |
| `pnpm --dir apps/desktop build && cargo test -p rig-desktop` | Typecheck/bundle Rig Desktop UI and run native Rust core unit tests |

Requirements: Rust (stable), Node with pnpm, Docker or Podman (4.9+), `curl` and `jq`.
Build the dashboard (`cd web && pnpm build`) before `cargo build`, because
the server embeds and pre-compresses `web/dist` at build time.
