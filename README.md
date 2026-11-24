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

## Local deploys & adopting existing containers

- **Deploy without Git (`rig up`):** Deploy a local project directory or a pre-compiled Linux ELF binary straight from your laptop with zero Git push required. Respects `.gitignore` and `.dockerignore`, deduplicates unchanged files and OCI layers via a content-addressed blob cache, streams `zstd`-compressed deltas over Rig's API, wraps static/dynamic binaries automatically into `scratch`/`distroless` OCI images, and performs a health-checked zero-downtime swap.
- **One-click adopt (`rig adopt`):** Discover running Docker/Podman containers, Docker Compose stacks, or Coolify/Dokploy workloads, preview the generated `rig.toml` and Podman Quadlet (`.container` / `.network`) units before touching anything, encrypt detected secrets into Rig's vault, preserve exact host bind mounts and volumes in-place, and hand off traffic with automatic rollback on health-check failure.

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

Requirements: Rust (stable), Node with pnpm, Docker or Podman (4.9+), `curl` and `jq`.
Build the dashboard (`cd web && pnpm build`) before `cargo build`, because
the server embeds and pre-compresses `web/dist` at build time.
