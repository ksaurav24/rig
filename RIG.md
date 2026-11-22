# Rig

> A lightweight, self-hosted deployment platform built in Rust, running on Podman or Docker. A Dokploy and Coolify alternative for small VPSes and homelabs, with a fraction of the RAM and a faster, friendlier developer experience.

Status: planning · Last updated: 2026-11-15 · "Rig" is a working name.

---

## 1. What we are making

Rig is an open, self-hosted PaaS. You install it on your own server (a VPS, a home server, a mini PC), connect a git repo or pick an app from the catalog, and Rig builds, deploys, secures (HTTPS), backs up and monitors it. You operate it from a web dashboard, a CLI on your laptop, or an AI assistant through MCP.

It is a Rust binary with a React dashboard, Caddy as the proxy, SQLite as its database, and Podman or Docker as the container engine.

## 2. Why it exists

1. **Dokploy and Coolify are heavy and slow.** Dokploy alone uses around 500–600 MB of RAM on a small VPS ([350 MB, about 630 MB after a v0.27.1+ regression](https://laoujin.github.io/Atlas/research/2026-06-08-self-hosted-paas-for-homelab-pr-previews-decision-framework-2026/dokploy/)), mostly from its own Node app, Postgres, Redis and Traefik. On a $5–$10 VPS that is memory your apps need.
2. **Day-to-day operation is slow.** Navigation, build logs and container terminals feel sluggish.
3. **Podman users have no good option.** Dokploy's [Podman request was closed](https://issues.ecosyste.ms/hosts/GitHub/repositories/Dokploy%2Fdokploy/issues?page=6) without being built.
4. **Security record.** Coolify had [11 disclosed vulnerabilities in January 2026](https://www.virtua.cloud/learn/en/concepts/coolify-vs-dokploy-self-hosted-paas). Safer defaults and a careful first-time setup matter.

## 3. Goal, mission, vision

- **Goal:** the lightest and fastest-to-operate way to deploy apps on your own server.
- **Mission:** make self-hosting feel as easy as a managed platform, without the memory tax or the lock-in.
- **Vision:** `git push` or `rig deploy` to a live HTTPS URL in about 60 seconds, on hardware as small as a $5 VPS.

## 4. Principles

1. **UX and DX come first.** Speed of operation is a feature.
2. **Small by default.** Every dependency has to justify its RAM.
3. **Podman and Docker are both first-class.** One `Runtime` trait over the Docker-compatible API.
4. **Rig is the source of truth.** It stores every app, stack and route in its own database and pushes config to other components. Container labels are only used to find Rig's own containers again after a restart.
5. **Secure by default.** Locked-down first-time setup, RBAC, masked and encrypted secrets, rootless where possible.
6. **One install command.** One Rig process plus Caddy. Helpers run only while needed.

## 5. Target users

- **Solo developers and indie hackers** with a $5–$20 VPS.
- **Small teams and agencies** hosting client apps, several clients per server.
- **Homelab users** self-hosting open-source apps (Jellyfin, n8n, Nextcloud, …) on their home network, over Tailscale, or publicly.
- **People moving off Dokploy or Coolify** because of RAM, speed or Podman.

## 6. Priorities

**P0 (MVP)**
- One-command install with a secure, guided first-time setup.
- Organizations → projects → apps, with RBAC similar to Dokploy.
- Podman and Docker runtimes.
- Deploy from GitHub: Dockerfile, auto-detected builds (Railpack), and prebuilt images.
- Compose stacks and a one-click app catalog.
- Managed databases: Postgres, MySQL, Redis, MongoDB.
- Caddy proxy with automatic HTTPS, custom domains, and three access modes: public, home network (LAN), Tailscale.
- Live build and runtime logs, web terminal into containers.
- Env vars and secrets (encrypted at rest, masked by default).
- Zero-downtime deploys (stateless apps) and one-click rollback.
- Backups to any S3-compatible storage (databases, volumes, Rig's own state).
- Monitoring: CPU, RAM, disk and network for the server and each app, with alerts.
- Safety on small servers: default memory limits, disk-full warnings.
- Self-update for Rig and Caddy.
- CLI that connects to one or many Rig servers.
- MCP server so AI tools can operate Rig.

**P1**
- PR preview environments with auto-cleanup.
- Scale-to-zero for idle apps.
- "Why did this build fail?" explainer (bring your own AI key).
- Migration from Dokploy and Coolify.
- Environments per project (staging, production) if not in P0, see §16.

**P2**
- Multi-server management from one dashboard (agents), unless pulled forward, see §16.
- OIDC / SSO.
- OpenTelemetry receiver (logs, metrics, traces).
- ARM / Raspberry Pi support and multi-arch builds.
- Snapshot-based rollbacks (btrfs, ZFS).
- WASM plugin system.

## 7. Tech stack

| Layer | Choice | Why |
|---|---|---|
| Backend | Rust + Tokio, [axum](https://github.com/tokio-rs/axum) | Low memory, safe, single binary |
| Database | SQLite (WAL) via [sqlx](https://github.com/launchbadge/sqlx) | No separate DB process |
| Job queue | In-process, SQLite-backed | Replaces Redis |
| Container API | [bollard](https://docs.rs/bollard) over the Docker-compatible API (Docker and Podman) | One code path for both engines |
| Compose | The `docker compose` program, pointed at the Docker or Podman socket | Full Compose spec without reimplementing it |
| Builds | Dockerfile; [Railpack](https://railpack.com) via BuildKit (built into Docker; a temporary BuildKit container on Podman) | Same auto-detect on both engines, no idle RAM |
| Proxy and TLS | [Caddy](https://caddyserver.com) driven by its [admin API](https://caddyserver.com/docs/api) | Atomic config, automatic HTTPS, Tailscale certs |
| Frontend | React + Vite, static files served by Rig | No Node runtime on the server |
| Frontend libs | TanStack Router and Query, shadcn/ui, Cmd+K palette | Fast, keyboard-first UX |
| Live data | Server-Sent Events | Logs, status, metrics |
| Web terminal | [xterm.js](https://xtermjs.org) over WebSocket | Shell into containers |
| API contract | [utoipa](https://github.com/juhaku/utoipa) OpenAPI → TypeScript client (dashboard) and Rust client (CLI) | No drift between API, UI and CLI |
| CLI | Rust, same workspace | One language, shared types |
| MCP | Built into Rig (HTTP) and `rig mcp` (stdio, via the CLI) | Works with Claude Code, Cursor, others |
| Backups | Native dump tools per database, [restic](https://restic.net) for volumes, SQLite backup API for Rig's own state | S3-compatible, encrypted |

## 8. Architecture

```
  Browser / CLI / AI (MCP)
          │
  ┌───────▼──────────────────────────────────────────────┐
  │ Caddy  (public certs · LAN internal CA · Tailscale)   │
  └───────┬──────────────────────────────────────────────┘
          │ routes pushed via admin API
  ┌───────▼──────────────────────────────────────────────┐
  │                 Rig (Rust binary)                      │
  │  HTTP API + UI │ Deploy engine │ Scheduler/jobs        │
  │  Auth + RBAC   │ Builds        │ Backups               │
  │  MCP server    │ Metrics       │ Self-update           │
  │                    SQLite                              │
  └───────┬──────────────────────────────────────────────┘
          │ Runtime trait (Docker-compatible API)
   ┌──────┴───────────┐
   │ Podman  │ Docker │   + docker compose, BuildKit (on demand)
   └──────────────────┘
```

### Key decisions

| # | Decision | Reason | Trade-off |
|---|---|---|---|
| 1 | Rust binary + Caddy | Low RAM, simple install | Rig must supervise Caddy |
| 2 | SQLite instead of Postgres + Redis | Biggest RAM win | Single writer; revisit for multi-node control plane |
| 3 | Static React, not Next.js | No Node server | No SSR (not needed) |
| 4 | Caddy over Traefik | Push config, on-demand TLS, Tailscale certs | No TCP/UDP routing; DNS-challenge needs a custom Caddy build |
| 5 | Both engines via the Docker-compatible API | One code path | Podman edge cases need CI against both |
| 6 | `docker compose` for stacks | Full spec day one | Compose-on-Podman edge cases; Rig records stacks itself |
| 7 | Installer picks the engine | Migrants keep Docker, new installs get Podman | Two install paths to test |
| 8 | "Needs root" mode for some Podman apps | Device passthrough, host network, low ports | A rootful socket; shown with a warning badge |
| 9 | Stateful apps deploy stop-then-start | Avoid two containers writing one volume | A few seconds of downtime; overridable |
| 10 | OpenAPI-generated clients | UI and CLI share types | Codegen step |

### Runtime notes

- **Installer:** Docker if already present, otherwise Podman; user can override. Supported: Debian 12+, Ubuntu 24.04+, Fedora/RHEL 9+ (Podman 4.9+).
- **Rootless Podman setup:** a `rig` system user with lingering enabled, the Podman user socket, Caddy as a systemd service allowed to bind 80/443. Apps listen on localhost ports; Caddy proxies to them.
- **Bind-mount ownership** on rootless Podman: use the `:U` flag and explain it in the UI.
- **Docker safety:** never mount the Docker socket into apps; warn when a template asks for it.

## 9. Access modes and TLS

Each app (and each domain on it) has a visibility: **public**, **LAN**, or **Tailscale**.

| Mode | How it gets HTTPS | Notes |
|---|---|---|
| Public | Let's Encrypt (HTTP or TLS challenge); wildcard via DNS challenge | DNS challenge needs Caddy built with a DNS provider module; Rig downloads and manages that build |
| LAN | Best: a real domain pointing at the LAN IP plus a DNS challenge, so certs are trusted everywhere. Fallback: Caddy's internal CA, with a guided "trust this certificate" page per device | Caddy restricts LAN apps to private IP ranges |
| Tailscale | Caddy gets certs for `*.ts.net` names from the local Tailscale daemon automatically | One MagicDNS name per machine: apps get a path or port by default; per-app tailnet hostnames are a later option |

## 10. Organizations, projects and RBAC

- **Hierarchy:** Organization → Project → App. An "app" is a git app, an image, a Compose stack, a catalog app, or a managed database.
- **Roles (like Dokploy):**
  - **Owner:** everything, including billing-free org settings, deleting the org, transferring ownership.
  - **Admin:** everything except owner-only actions.
  - **Member:** only the projects they are granted, with per-permission toggles: create projects, create/delete apps, deploy, read secrets, open terminals, manage domains, manage backups, manage git providers, create API tokens.
- **API tokens** carry a role and scope, and are used by the CLI, CI and MCP.
- **Audit log** of who did what (deploys, secret reads, terminal sessions, role changes).

## 11. Features

### Deploy
- **Sources:** a GitHub repo, a prebuilt image, a Compose file, a catalog template, or a local folder via `rig deploy`.
- **Build paths:**
  - On the server, memory-capped, one build at a time.
  - In CI: we ship a GitHub Actions workflow that builds and pushes to a registry; Rig pulls.
  - From the laptop: `rig deploy --local-build` streams the image to the server.
- **Git triggers:** a GitHub App via GitHub's manifest flow on public servers; polling for private servers; a deploy-hook URL for any CI.
- Zero-downtime deploys for stateless apps (health check the new container, then switch the route). One-click rollback. Deploy history with logs.

### App catalog
- **Template format:** a Compose file plus metadata (name, icon, description, inputs, generated passwords, default domain and visibility).
- **Where templates live:** a separate public git repo; Rig fetches it.
- **Seeding:** import Coolify's templates (Apache-2.0); check Dokploy's template license.
- **Testing:** every template is tested in CI on Docker and Podman.

### Managed databases
- **Supported:** Postgres, MySQL, Redis and MongoDB, with version selection.
- **Connection:** connection strings are injected into linked apps.
- **External access:** optional, via a direct port with an IP allowlist (Caddy does not route raw TCP).
- **Backups:** native dumps (`pg_dump`, `mysqldump`, `mongodump`, Redis RDB). Scheduled, sent to S3, with a visible restore button.

### Backups
- **Destinations:** any S3-compatible storage (AWS, R2, B2, MinIO, Wasabi, …), configured by the user. Local disk is optional too.
- **What gets backed up:** databases, volumes (restic, encrypted), and Rig's own SQLite state.
- **Scheduling:** schedules and retention rules, plus a "test restore" action and notifications on failure.

### Monitoring
- **Metrics:** CPU, RAM, disk and network for the server and for each app.
- **Collection and storage:** sampled every ~10 s in memory, stored as rollups in SQLite (1-minute for 7 days, 1-hour for 90 days).
- **Alerts:** high memory, disk nearly full, a crash-looping app, a failed backup, an expiring certificate. Delivered by Discord, Slack, Telegram, email and webhooks.
- **Small-server safety:** default memory limits per app and a warning before the server runs out of disk.

### Config and secrets
- Env vars per project and app. `.env` paste and import.
- Secrets are encrypted at rest (a master key in a 0600 file), masked by default, and every reveal is logged.

### Fast operation (DX targets)
- **Navigation:** the Cmd+K palette reaches any app, deploy, log or terminal in two keystrokes. Every page has a link you can share.
- **Build logs:** streaming starts in under 1 s, with search, ANSI colors, collapsible steps and the error jumped to automatically.
- **Terminal:** one click (or `rig exec`) opens a shell in any container.
- **Targets (to measure):** dashboard loads in under 1 s on a $5 VPS; any action responds visibly within 100 ms.

## 12. First-time setup and security

1. **Install:** `curl … | sh` checks the OS, ports 80/443, RAM and disk, and installs the engine and Caddy.
2. **Setup link:** the installer prints a one-time setup link with a token (single use, expires in 30 minutes). The dashboard refuses everything else until setup is done, so nobody can claim the server before you.
3. **Wizard:**
   - Admin account (password plus TOTP or passkey).
   - Dashboard domain and visibility.
   - Email for certificates.
   - Engine confirmation.
   - Optional S3 backup target.
   - Optional Tailscale.
4. **Checks:** DNS points to this server, the firewall is sane, SSH password login is warned about. The dashboard then moves to HTTPS.
5. **Defaults:**
   - Sessions with a short life and logout from all devices.
   - Rate-limited login.
   - The dashboard can be set to LAN-only or Tailscale-only.
   - All secrets encrypted at rest.

## 13. Self-update

- **Triggers:** `rig update` or a dashboard button. Stable and beta channels.
- **Release checks:** every release is signed and checked (checksum and signature) before installing.
- **Update steps:**
  1. Back up the SQLite database.
  2. Swap the binary.
  3. Run schema migrations.
  4. Restart via systemd.
  5. Health check, rolling back automatically if it fails.
- **Caddy:** updated the same way, with a managed download.
- **Images:** a notice when a catalog app or database image has a newer version.

## 14. CLI and MCP

### CLI (`rig`)
- `rig login https://server`: browser sign-in, with the token stored in the OS keychain.
- **Multiple servers:** like kubectl contexts.
  - `rig context add/use/list` to manage them.
  - `--server` on any command to target one.
- **Commands:**
  - `rig deploy`: from git, from a local folder (uploads the source, respecting `.gitignore`), or `--local-build`.
  - `rig logs -f`, `rig rollback`, `rig exec`, `rig env`, `rig status`, `rig db`, `rig backup`, `rig update`.
- **Config as code:** a `rig.toml` in the repo.

### MCP
- **Two ways to connect:**
  - Built into Rig (HTTP transport).
  - `rig mcp` (stdio, through the CLI's current context), for local tools like Claude Code.
- **Tools:** list apps, deploy, read logs, roll back, status and metrics, manage env vars (secrets stay masked unless the token allows reading), run backups.
- **Safety:**
  - Every MCP call uses a scoped API token and is audited.
  - Destructive actions need a token with that permission.
  - Log content is treated as untrusted (prompt-injection risk) and clearly separated from instructions.

## 15. License

**Recommended: [FSL-1.1-ALv2](https://fsl.software) (Functional Source License, Apache 2.0 future).**

- **Allowed:** using Rig for anything, including commercial use: agencies hosting client projects, companies running their own apps, professional services built on Rig.
- **Forbidden:** offering Rig, or a fork of it, as a competing commercial product or hosted service.
- **Conversion:** each release automatically becomes Apache 2.0 two years after it ships.

**Alternative:** [Elastic License 2.0](https://www.elastic.co/licensing/elastic-license). It forbids offering the software as a hosted or managed service permanently, with no conversion. It's stricter, and the community likes it less.

**Things to know:**
- **Not OSI open source:** neither license is "open source" by the OSI definition. The accurate terms are "source-available" or "fair source", so say that honestly in the README.
- **Agency FAQ:** write one that says agencies may give clients logins to their own apps. That is hosting clients' apps, not reselling Rig.
- **Contributions:** use a CLA (or a DCO plus an explicit license grant) so the license can be changed later.
- **Legal review:** get a lawyer to look at it before the public launch.

## 16. Open questions

1. **Multiple servers:** does "multiple servers" mean the CLI talks to several independent Rig installs (P0 here), or one Rig dashboard that deploys onto several servers (agents, currently P2)?
2. **Environments:** should projects have environments (staging, production) in the MVP, or later with PR previews?
3. **Name:** decide before the public alpha.

## 17. Risks

- **Scope:** the MVP is now large (two engines, three access modes, four databases, RBAC, backups, monitoring, CLI, MCP, catalog). Build it as thin vertical slices and hold the line on P1/P2.
- **Podman edge cases:** Compose and the compatibility API differ from Docker in places. CI runs everything against both.
- **Caddy is a second process** that Rig must supervise and recover. Custom builds are needed for DNS challenges.
- **Competition:** [Komodo](https://github.com/moghtech/komodo) and [Temps](https://temps.sh/blog/dokploy-alternatives-2026) are already in the Rust space. We win on UX, speed, Podman, the catalog and AI.
- **The license may put off some contributors.** Explain it plainly.

## 18. Targets (estimates, to be measured)

| Metric | Dokploy (reported) | Rig target |
|---|---|---|
| Platform idle RAM | ~500–600 MB | Rig ~15–30 MB + Caddy ~25–40 MB (+ Docker daemon ~50–100 MB if Docker; ~0 for Podman) |
| Idle CPU | ~1% | under 1% |
| First deploy | varies | under 60 s for a simple app |
| Dashboard load | slow | under 1 s |
| Install | multi-step | one command |

Publish an honest benchmark against Dokploy early.

## 19. Next steps

1. Answer the open questions in §16.
2. Design the data model: organizations, users, memberships, projects, apps, deployments, builds, domains, env vars, databases, backups, metrics, API tokens, audit log, servers.
3. Define the `Runtime` and `Proxy` traits and the Caddy config flow for the three access modes.
4. Set up the repo: a Rust workspace (server, CLI, shared API types) and the Vite app, with the OpenAPI codegen pipeline.
5. Build a thin vertical slice: install, set up, deploy one Dockerfile app to a public HTTPS URL on both engines, with logs.
