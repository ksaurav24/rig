# Rig Pre-Release Security Audit Report

| Field | Value |
|---|---|
| **Target** | Rig (`crates/rig-server`, `crates/rig-api`, `crates/rig-cli`, `web/`, `packaging/`) |
| **Audit Date** | 2026-11-22 |
| **Reviewed Commit** | `fa258631dfde84d8af035b564ce68e4cf81b3477` (with uncommitted CLI overhaul changes in worktree) |
| **Methodology** | Defensive, source-first security review across trust boundaries, auth/session/TOTP state machines, secret storage, SSRF, build/container isolation, HTTP/proxy controls, and packaging |
| **Findings Summary** | **0 Critical · 7 High · 7 Medium · 3 Low** |

---

## 1. Executive Summary

Rig is a self-hosted deployment platform that holds high-value credentials (environment secrets, managed database passwords, GitHub App private keys, API tokens, and container engine socket access) directly on the host server.

The codebase demonstrates strong foundational engineering: Argon2id password hashing, XChaCha20-Poly1305 AEAD encryption (`Vault`) with strict `0600` key checks, type-safe `Auth<R>` Axum extractors enforcing role hierarchy and CSRF tokens, parameterized SQLx queries, `Command` argument-vector execution without shell interpolation, and strict `systemd` service sandboxing.

Before public release, however, **7 High** and **7 Medium** security issues should be addressed. The highest-risk issues involve:
1. **SSRF via alert channels** reaching the unauthenticated local Caddy Admin API (`127.0.0.1:2019`), loopback services, and cloud metadata.
2. **Host-side symlink traversal exposure** during `git clone` + `railpack prepare` running directly as the `rig` OS user (owner of `master.key`, `rig.db`, and backups).
3. **API token persistence & overly broad scope**, allowing a leaked short-lived token to mint permanent backdoor tokens, modify team/network settings, and survive password resets.
4. **Unrate-limited password verification** on TOTP management endpoints.
5. **Weakly hashed TOTP recovery codes** (unsalted SHA-256 over a ~39.6-bit space) and **plaintext webhook/alert signing secrets** in SQLite.
6. **Unauthenticated `/api/docs`** loading unpinned, non-SRI third-party JavaScript from `cdn.jsdelivr.net` on Rig's privileged origin with no CSP.
7. **Container-to-host Caddy reachability** allowing compromised app containers to bypass project network isolation and `Access::Lan` controls.

### Findings Overview

| ID | Severity | Category | Summary | Primary Locations |
|---|---|---|---|---|
| **RIG-SEC-01** | High | SSRF | Alert channels allow `127.0.0.1`, private IPs, and cloud metadata — can stop Caddy (`127.0.0.1:2019/stop`) and probe loopback services | `crates/rig-server/src/alerts.rs`, `routes/alerts.rs` |
| **RIG-SEC-02** | High | Host File Disclosure | `git clone` checks out symlinks verbatim before host-side `railpack prepare` runs as `rig` user | `crates/rig-server/src/deploy.rs`, `railpack.rs` |
| **RIG-SEC-03** | High | Auth / Token Lifecycle | API tokens can self-mint permanent tokens, call sensitive admin/network routes, and survive password resets | `crates/rig-server/src/routes/auth.rs`, `routes/members.rs`, `routes/system.rs`, `main.rs` |
| **RIG-SEC-04** | High | Brute Force / DoS | `routes/account.rs::verify_password()` has no rate limiting or failure auditing on TOTP endpoints | `crates/rig-server/src/routes/account.rs` |
| **RIG-SEC-05** | High | Secrets at Rest | Weak SHA-256 hashing of 39.6-bit TOTP recovery codes; plaintext `apps.webhook_secret` and `alert_channels.signing_secret` in SQLite | `crates/rig-server/src/totp.rs`, `routes/account.rs`, `routes/apps.rs`, `routes/alerts.rs` |
| **RIG-SEC-06** | High | Unauthenticated API / Supply Chain XSS | Unauthenticated `/api/docs` loads unpinned `@scalar/api-reference` from `cdn.jsdelivr.net` on Rig's origin without CSP | `crates/rig-server/src/routes/mod.rs` |
| **RIG-SEC-07** | High | Network Isolation | Host-networked Caddy accepts traffic from container bridges (`172.16.0.0/12`, `10.0.0.0/8`), bypassing project isolation and `Access::Lan` | `packaging/install.sh`, `proxy.rs`, `access.rs` |
| **RIG-SEC-08** | Medium | Credential Exposure | Redis password passed in container argv (`--requirepass <pw>`), visible in host `ps` and `inspect` | `crates/rig-server/src/databases.rs` |
| **RIG-SEC-09** | Medium | State Machines & TOCTOU | Stateless 2FA login, non-expiring temp passwords, missing session rotation, `delete_project` destructive ordering bug, and check-then-act races | `routes/auth.rs`, `routes/members.rs`, `routes/projects.rs`, `routes/apps.rs`, `routes/databases.rs`, `routes/account.rs`, `main.rs` |
| **RIG-SEC-10** | Medium | Session Revocation | Active terminal WebSockets and SSE log streams never re-check session/token revocation or role demotion; no concurrency cap | `crates/rig-server/src/routes/terminal.rs`, `routes/apps.rs` |
| **RIG-SEC-11** | Medium | HTTP Trust Boundaries | `check_origin` ignores scheme (`http` vs `https`) and trusts `X-Forwarded-Host` unconditionally; `is_https` trusts `X-Forwarded-Proto` unconditionally; `real_ip` reads only first `X-Forwarded-For` header | `crates/rig-server/src/auth.rs` |
| **RIG-SEC-12** | Medium | SSRF / Git | Git clone/poller allows internal IPs, leaves `http.followRedirects` enabled, and uses protocol denylist instead of allowlist | `crates/rig-server/src/deploy.rs`, `github.rs` |
| **RIG-SEC-13** | Medium | Availability / Rate Limiting | `login_attempts` and expired `sessions` are never pruned (disk growth DoS); per-email lockout enables trivial admin lockout DoS | `crates/rig-server/src/auth.rs`, `routes/auth.rs` |
| **RIG-SEC-14** | Medium | Container & Backup Hardening | Missing container `pids_limit`; rootful Podman triggers `privileged: true` BuildKit; DB backups stored unencrypted on disk | `crates/rig-server/src/runtime/docker.rs`, `backups.rs` |
| **RIG-SEC-15** | Low | HTTP Headers / Caching | Missing HSTS on HTTPS responses; `Cache-Control: public` sent on private user avatars (`is_public = 0`) + 401/404 existence oracle | `crates/rig-server/src/routes/mod.rs`, `routes/org.rs` |
| **RIG-SEC-16** | Low | Content Validation | Bypassable SVG blocklist sanitizer (currently mitigated by `sandbox` CSP + `<img>`); arbitrary external `http://`/`https://` avatar/logo URLs allow tracking beacons | `crates/rig-server/src/routes/org.rs`, `dashboard.rs` |
| **RIG-SEC-17** | Low | Operational Hygiene | Setup token captured in `journald` via `eprintln!`; unbounded `github::STATES` map; GitHub hook accepts `X-Gitlab-Token`; unconditional `/run/tailscale` mount; version disclosed in `/api/v1/setup` | `main.rs`, `github.rs`, `routes/github.rs`, `routes/auth.rs`, `packaging/install.sh` |

---

## 2. Existing Security Controls to Preserve

The following controls are well implemented and should be preserved during refactoring:

- **Password hashing (`crypto.rs:131-146`, `routes/auth.rs:68-78, 288-295`):** Argon2id (`Argon2::default()`, random salt, PHC string format), 10–256 char/byte length bounds preventing Argon2 CPU DoS, and constant-time dummy hash verification for missing/disabled users.
- **Vault AEAD encryption (`crypto.rs:38-110`):** XChaCha20-Poly1305 with fresh random 24-byte nonces per encryption and strict startup refusal if `master.key` has group/world permissions (`mode & 0o077 != 0`).
- **Role-typed extractors (`auth.rs:58-192`):** Compile-time `Auth<Viewer|Developer|Admin|Owner>` extraction with token role capped at `token_role.min(member_role)`, `HttpOnly` + `SameSite=Strict` cookies, and constant-time `x-rig-csrf` validation on cookie writes.
- **SQL & command safety:** Compile-time literal parameterized SQLx queries throughout; subprocesses invoked via `Command::new` argv arrays with `--` option terminators (no shell interpolation of user inputs).
- **Container & build basics:** Loopback-only container port bindings (`127.0.0.1`), `no-new-privileges`, named Docker volumes only (host bind mounts prohibited), pinned Railpack binary SHA-256 and pinned BuildKit frontend digest, and disabled HTTP redirects in CLI and alert HTTP clients.
- **Systemd unit hardening (`packaging/rig.service`):** `UMask=0077`, `NoNewPrivileges=yes`, `ProtectSystem=strict`, `ReadWritePaths=/var/lib/rig`, `ProtectHome=yes`, `PrivateTmp=yes`, `PrivateDevices=yes`, `CapabilityBoundingSet=`, `RestrictNamespaces=yes`, `LockPersonality=yes`.

---

## 3. Complete Endpoint & Authentication Matrix

| Route | Method(s) | Auth / Min Role | Accepts API Token? | Notes |
|---|---|---|---|---|
| `/api/docs` | `GET` | **None** | N/A | **RIG-SEC-06**: Unauthenticated, no CSP, unpinned CDN script on Rig origin |
| `/api/openapi.json` | `GET` | **None** | N/A | **RIG-SEC-06**: Unauthenticated full API schema |
| `/api/v1/setup` | `GET` | **None** | N/A | Public status + org branding; leaks `version` and `env_label` |
| `/api/v1/setup` | `POST` | Setup token | N/A | Origin check + IP rate limit (10/15m); atomic token consume |
| `/api/v1/auth/login` | `POST` | Email + Password (+ TOTP) | N/A | **RIG-SEC-09a/13**: Stateless 2FA, silent step-1, email lockout DoS |
| `/api/v1/auth/logout` | `POST` | `Viewer` | Yes | Deletes current cookie session; no-op for API token |
| `/api/v1/auth/me` | `GET` | `Viewer` | Yes | Returns `csrf_token` for browser sessions |
| `/api/v1/auth/password` | `POST` | `Viewer` (**Session only**) | No | Rate-limited; revokes other sessions, does **not** rotate current session |
| `/api/v1/tokens` | `GET`, `POST` | `Viewer` | **Yes** | **RIG-SEC-03**: API token can mint unlimited permanent tokens |
| `/api/v1/tokens/{id}` | `DELETE` | `Viewer` | **Yes** | Scoped to own `(org_id, user_id)` |
| `/api/v1/audit` | `GET` | `Admin` | Yes | Last 200 entries for org |
| `/api/v1/account/profile` | `PATCH` | `Viewer` | Yes | **RIG-SEC-16**: Allows external `http://`/`https://` avatar URLs |
| `/api/v1/account/avatar` | `POST`, `DELETE` | `Viewer` | Yes | Stores private media (`is_public = 0`) |
| `/api/v1/account/totp` | `GET` | `Viewer` | Yes | Status + recovery code count |
| `/api/v1/account/totp/setup` | `POST` | `Viewer` (**Session only**) | No | **RIG-SEC-04/09c**: Unrate-limited password check; no `totp_pending` TTL |
| `/api/v1/account/totp/enable` | `POST` | `Viewer` (**Session only**) | No | Rate-limited TOTP code; does not rotate current session |
| `/api/v1/account/totp/disable` | `POST` | `Viewer` (**Session only**) | No | **RIG-SEC-04**: Unrate-limited password check runs before TOTP rate limit |
| `/api/v1/account/totp/recovery-codes` | `POST` | `Viewer` (**Session only**) | No | **RIG-SEC-04/05**: Unrate-limited password check; weak SHA-256 storage |
| `/api/v1/members` | `GET`, `POST` | `Admin` | **Yes** | **RIG-SEC-03/09b**: API token allowed; temp passwords never expire |
| `/api/v1/members/{user_id}` | `PATCH`, `DELETE` | `Admin` | **Yes** | **RIG-SEC-03/09e**: API token allowed; role check outside tx |
| `/api/v1/members/{user_id}/reset-password` | `POST` | `Admin` | **Yes** | **RIG-SEC-03/09b**: API token allowed; does not revoke `api_tokens` |
| `/api/v1/org` | `PATCH` | `Admin` | Yes | Updates org branding (`logo_url` allows external URLs) |
| `/api/v1/org/logo` | `POST`, `DELETE` | `Admin` | Yes | Stores public media (`is_public = 1`) |
| `/api/v1/media/{id}` | `GET` | Public if `is_public=1`, else `Viewer` | Yes | **RIG-SEC-15**: `Cache-Control: public` even on private avatars; 401/404 oracle |
| `/api/v1/system` | `GET` | `Viewer` | Yes | Host/engine/proxy/app counts |
| `/api/v1/system/network` | `GET` | `Viewer` | Yes | LAN IPs, Tailscale DNS/IPs, dashboard domain/access |
| `/api/v1/system/dashboard-access` | `PUT` | `Owner` | **Yes** | **RIG-SEC-03**: API token can lower access restriction to `public` |
| `/api/v1/system/dashboard-domain` | `PUT` | `Owner` | **Yes** | **RIG-SEC-03**: API token can hijack dashboard domain |
| `/api/v1/system/metrics` | `GET` | `Viewer` | Yes | Host metrics series |
| `/api/v1/projects` | `GET` (`Viewer`), `POST` (`Developer`) | `Viewer` / `Developer` | Yes | Org-scoped |
| `/api/v1/projects/{id}` | `PATCH` (`Developer`), `DELETE` (`Admin`) | `Developer` / `Admin` | Yes | **RIG-SEC-09d**: `DELETE` removes app containers/volumes before checking creating DBs |
| `/api/v1/apps` | `GET` (`Viewer`), `POST` (`Developer`) | `Viewer` / `Developer` | Yes | **RIG-SEC-05/12**: Plaintext `webhook_secret`; internal Git URLs allowed |
| `/api/v1/apps/{id}` | `GET` (`Viewer`), `PATCH` (`Developer`), `DELETE` (`Admin`) | Role-tiered | Yes | Org-scoped; `DELETE` has deploy check TOCTOU |
| `/api/v1/apps/{id}/deploy`, `/stop` | `POST` | `Developer` | Yes | **RIG-SEC-09e**: Deploy check outside tx |
| `/api/v1/apps/{id}/webhook`, `/webhook/rotate` | `GET`, `POST` | `Developer` | Yes | Audited; **RIG-SEC-05**: plaintext DB storage |
| `/api/v1/apps/{id}/logs` | `GET` (SSE) | `Viewer` | Yes | **RIG-SEC-10**: No mid-stream auth revocation check |
| `/api/v1/apps/{id}/metrics` | `GET` | `Viewer` | Yes | Org-scoped |
| `/api/v1/apps/{id}/terminal` | `GET` (WS) | `Developer` | Yes | **RIG-SEC-10/11**: No mid-session revocation check; scheme-less `Origin` check |
| `/api/v1/apps/{id}/deployments` | `GET` | `Viewer` | Yes | Org-scoped |
| `/api/v1/deployments/{id}` | `GET` | `Viewer` | Yes | Org-scoped |
| `/api/v1/deployments/{id}/rollback` | `POST` | `Developer` | Yes | Org-scoped |
| `/api/v1/deployments/{id}/logs` | `GET` (SSE) | `Viewer` | Yes | Org-scoped |
| `/api/v1/apps/{id}/env` | `GET` (`Viewer`), `PUT` (`Developer`) | `Viewer` / `Developer` | Yes | Secrets masked on `GET`; encrypted in DB |
| `/api/v1/apps/{id}/env/{key}` | `DELETE` | `Developer` | Yes | Audited |
| `/api/v1/apps/{id}/env/{key}/reveal` | `POST` | `Developer` | Yes | Audited before decrypting |
| `/api/v1/apps/{id}/domains` | `GET` (`Viewer`), `POST` (`Developer`) | `Viewer` / `Developer` | Yes | Hostname validated; port/TLS validated |
| `/api/v1/apps/{id}/domains/{domain_id}` | `DELETE` | `Developer` | Yes | Org-scoped |
| `/api/v1/databases`, `/engines`, `/{id}` | `GET` (`Viewer`), `POST` (`Developer`), `DELETE` (`Admin`) | Role-tiered | Yes | **RIG-SEC-08/09e**: Redis argv password; delete TOCTOU |
| `/api/v1/databases/{id}/start`, `/stop`, `/reveal`, `/backups` | `POST` / `GET` | `Developer` / `Viewer` | Yes | Reveal audited before decrypt |
| `/api/v1/databases/{id}/metrics` | `GET` | `Viewer` | Yes | Org-scoped |
| `/api/v1/backups/{id}`, `/download`, `/restore` | `DELETE`, `GET`, `POST` | `Admin` | Yes | **RIG-SEC-09e/14**: Restore lacks concurrency guard; dumps unencrypted on disk |
| `/api/v1/github` | `GET` (`Viewer`), `DELETE` (`Admin`) | `Viewer` / `Admin` | Yes | Clears cached installation tokens on disconnect |
| `/api/v1/github/manifest`, `/manifest/complete` | `POST` | `Admin` | **Yes** | **RIG-SEC-03/17**: API token bypasses `Origin` check; unbounded `STATES` map |
| `/api/v1/github/repos`, `/repos/{o}/{r}/branches` | `GET` | `Developer` | Yes | Scoped to org's GitHub App |
| `/api/v1/alerts/channels` | `GET`, `POST` | `Admin` | **Yes** | **RIG-SEC-01/05**: SSRF target URL; plaintext `signing_secret` |
| `/api/v1/alerts/channels/{id}`, `/{id}/test` | `PATCH`, `DELETE`, `POST` | `Admin` | **Yes** | **RIG-SEC-01**: Triggers immediate outbound POST with error feedback |
| `/api/v1/hooks/apps/{id}` | `POST` | App webhook secret | N/A | Bypasses `access::guard`; HMAC-SHA256 or `X-Gitlab-Token` |
| `/api/v1/hooks/github/{org_id}` | `POST` | Org GitHub webhook secret | N/A | Bypasses `access::guard`; **RIG-SEC-17**: also accepts `X-Gitlab-Token` |

---

## 4. Password, Secret, and Token Storage Inventory

| Secret / Credential | Generation | Storage Mechanism | Status | Required Upgrade |
|---|---|---|---|---|
| User passwords | User-supplied (10–256 chars/bytes) or `random_token(15)` | Argon2id PHC string (`users.password_hash`) | **Good** | Add `must_change_password` + expiry for admin-generated temp passwords |
| Session tokens | `random_token(32)` (256-bit) | SHA-256 hex (`sessions.token_hash`), `HttpOnly`/`SameSite=Strict` | **Good** | Rotate on password/2FA changes; prune expired rows; add idle timeout & session management |
| CSRF tokens | `random_token(24)` (192-bit) | Plaintext in `sessions.csrf_token` | **Good** | Rotate alongside session token |
| API tokens | `rig_` + `random_token(32)` (256-bit) | SHA-256 hex (`api_tokens.token_hash`) | **Needs hardening** | Block `Via::ApiToken` from minting new tokens or calling admin identity/network APIs; revoke on password resets |
| Setup token | `random_token(24)` (192-bit), 30m TTL | SHA-256 hex (`settings.setup_token_hash`) | **Minor gap** | Avoid writing token to `journald` via `eprintln!` under systemd |
| TOTP secrets | 20 random bytes (160-bit Base32) | Vault XChaCha20-Poly1305 (`users.totp_secret`, `totp_pending`) | **Needs hardening** | Add expiry TTL to `totp_pending` |
| **TOTP recovery codes** | 10 × 8 chars from 31-char alphabet (**39.6 bits**) | **Unsalted fast SHA-256** (`Sha256("{user_id}:{code}")`) | **Weak (High)** | Upgrade to ≥ 128-bit codes OR Argon2id / Vault-keyed HMAC-SHA256 |
| App env vars | User-supplied | Vault XChaCha20-Poly1305 (`env_vars.value_enc`) | **Good** | Keep as-is |
| Database passwords | 24 random bytes hex (192-bit) | Vault XChaCha20-Poly1305 (`databases.password_enc`) | **Medium gap** | Remove Redis `--requirepass <pw>` from container argv |
| GitHub App keys/secrets | From GitHub manifest conversion | Vault XChaCha20-Poly1305 (`github_apps.*`); install tokens RAM-only | **Good** | Keep as-is |
| Alert channel URLs | User-supplied | Vault XChaCha20-Poly1305 (`alert_channels.url_enc`) | **Good** | Add SSRF IP validation on connect |
| **App webhook secrets** | `random_token(24)` (192-bit) | **Plaintext** (`apps.webhook_secret`) | **Weak (High)** | Encrypt with `Vault` |
| **Alert signing secrets** | `random_token(24)` (192-bit) | **Plaintext** (`alert_channels.signing_secret`) | **Weak (High)** | Encrypt with `Vault` |
| Database backups | Engine dump output | **Unencrypted** files on disk (`0600`) | **Hardening (Med)** | Optionally stream-encrypt with `Vault` |
| CLI config tokens | Saved by `rig login` | Plaintext TOML (`0600`, `~/.config/rig/config.toml`) | **Acceptable** | Standard for CLI tools; atomic `0600` temp-file write is good |

---

## 5. Detailed Findings

### High Severity

#### RIG-SEC-01: SSRF in Alert Channels Reaches Unauthenticated Caddy Admin API, Loopback Services, and Cloud Metadata
- **Category:** Server-Side Request Forgery (SSRF)
- **Trust Boundary:** `Admin` (or leaked Admin API token) → host loopback services (`127.0.0.1:2019` Caddy Admin, `127.0.0.1:7460` Rig API, `127.0.0.1:<port>` internal apps/DBs) and internal/cloud metadata networks (`169.254.169.254`)
- **Locations:**
  - `crates/rig-server/src/alerts.rs:56-71` (`validate_url`), `:188-216` (`deliver`), `:238` (test explicitly permitting `http://127.0.0.1:9000/hook`)
  - `crates/rig-server/src/routes/alerts.rs:95-138` (`create_channel`), `:202-226` (`test_channel`)
  - `crates/rig-server/src/proxy.rs:260-265` (Caddy admin bound to `127.0.0.1:2019` without authentication)
- **Root Cause:** `alerts::validate_url` checks only URL syntax, `http`/`https` scheme, non-empty host, and absence of userinfo. It does not reject loopback, link-local, RFC1918/ULA, or CGNAT IP literals, nor does `alerts::deliver` validate resolved IP addresses before opening the TCP socket. `test_channel` immediately performs an outbound `POST` and returns the raw error (`format!("{e:#}")`) or HTTP status code to the caller.
- **Impact:**
  - **Denial of Service against reverse proxy:** An `Admin` (who is *not* `Owner` and cannot modify proxy settings) can register `http://127.0.0.1:2019/stop` and call `/test`, causing Caddy's unauthenticated admin API to shut down and taking Rig and every hosted app offline.
  - **Internal service & metadata probing:** Attackers can probe loopback-only containers, cloud instance metadata (`169.254.169.254`), or internal LAN/tailnet services, and invoke `http://127.0.0.1:7460/...` with loopback peer identity (bypassing `lan`/`tailscale` `access::guard`).
- **Smallest Effective Fix:**
  1. Implement a custom `reqwest::dns::Resolve` wrapper on `alerts::CLIENT` that resolves the host, rejects any non-global IP (`is_loopback()`, `is_private()`, `is_link_local()`, `is_unspecified()`, `is_multicast()`, IPv4 `100.64.0.0/10`, IPv4-mapped IPv6 equivalents, and `fc00::/7`), and returns only verified public IPs (defeating DNS rebinding).
  2. Reject private/loopback IP literals up front in `validate_url` as well. If operators need LAN webhook receivers, gate them behind an explicit server CLI/env allowlist (e.g., `--allow-private-alert-hosts`).
  3. Return generic failure messages from `test_channel` (e.g., `"connection failed"` / `"non-2xx response"`) instead of raw transport error strings.

---

#### RIG-SEC-02: Host Filesystem Symlink Traversal Risk via Git Checkout + Host-Side `railpack prepare`
- **Category:** Path / Symlink Traversal, Host Secret Disclosure
- **Trust Boundary:** Untrusted Git repository (`Developer` or anyone with push access to an `auto_deploy` repo) → host filesystem readable by the `rig` OS user (`/var/lib/rig/master.key`, `/var/lib/rig/rig.db`, `/etc/rig/rig.env`, `/var/lib/rig/backups/`)
- **Locations:**
  - `crates/rig-server/src/deploy.rs:690-704` (`git clone` without `core.symlinks=false`)
  - `crates/rig-server/src/deploy.rs:748-765` (`try_exists` following symlinks + invoking `railpack::prepare` on host)
  - `crates/rig-server/src/railpack.rs:144-179` (`railpack prepare` executing directly on host as `rig` user, logging stdout/stderr to deployment logs)
  - `crates/rig-server/src/runtime/docker.rs:885-907` (`pack_context` enforces symlink containment only for Dockerfile builds)
- **Root Cause:** While `pack_context` explicitly disables symlink following (`tar.follow_symlinks(false)`) for Dockerfile builds, `git clone` runs with Linux's default `core.symlinks=true`. For repositories without a `Dockerfile`, Rig executes `railpack prepare` directly on the host as user `rig` against the cloned directory — the same OS user that owns `master.key` (mode `0600`), `rig.db`, and unencrypted database backups. Furthermore, `deploy.rs:748` uses `tokio::fs::try_exists(checkout.join(&app.dockerfile_path))`, which follows symlinks outside `checkout`.
- **Impact:** A malicious commit containing symlinks pointing to `/var/lib/rig/master.key`, `/var/lib/rig/rig.db`, `/etc/rig/rig.env`, or backup dumps violates Rig's symlink-containment boundary. Unless the external `railpack` v0.43.0 binary strictly confines every file open with `os.Root`/no-follow semantics, any symlinked config/version/Procfile read by Railpack can disclose host secret contents into deployment logs (readable by `Viewer`) or embed them into `railpack-plan.json` and the built image.
- **Smallest Effective Fix:**
  1. Pass `-c core.symlinks=false` to `git clone` in `deploy.rs:693-703`, or walk `checkout` immediately after clone and reject/unlink any symlink whose resolved target escapes `checkout` (and reject symlinks for `dockerfile_path` via `symlink_metadata`).
  2. Longer term, run `railpack prepare` inside a minimal unprivileged container with only `checkout` (read-only) and `plan_dir` mounted, never with host access to `RIG_DATA_DIR`.

---

#### RIG-SEC-03: API Tokens Can Self-Mint Permanent Tokens, Access Sensitive Admin/Network Endpoints, and Survive Password Resets
- **Category:** Broken Access Control / Credential Lifecycle
- **Trust Boundary:** Leaked API token (CLI, CI, MCP) → permanent account persistence, org membership takeover, network access downgrade
- **Locations:**
  - `crates/rig-server/src/routes/auth.rs:492-538` (`POST /api/v1/tokens` allows `Via::ApiToken`, no token count limit)
  - `crates/rig-server/src/routes/members.rs:119-323` (`add_member`, `update_member`, `remove_member`, `reset_password` allow `Via::ApiToken`)
  - `crates/rig-server/src/routes/system.rs:156-227` (`set_dashboard_access`, `set_dashboard_domain` allow `Via::ApiToken`)
  - `crates/rig-server/src/routes/github.rs:104-158` (`POST /api/v1/github/manifest` allows `Via::ApiToken`, invalidating its line-119 comment assuming `Origin` was already checked)
  - `crates/rig-server/src/routes/members.rs:306` & `crates/rig-server/src/main.rs:353` (password resets delete `sessions`, leaving `api_tokens` valid)
- **Root Cause:** Only `change_password` and `totp_*` check `auth.via == Via::Session`. All other control-plane routes accept bearer API tokens equally with interactive browser sessions, and emergency password resets fail to clear `api_tokens`.
- **Impact:**
  - **Persistence:** A leaked 1-day API token can call `POST /api/v1/tokens` with `expires_in_days: null` to mint unlimited non-expiring tokens at the same role. Revoking the leaked token does not evict the attacker.
  - **Blast radius:** A leaked Admin/Owner token can add a new Owner account, reset another member's password (receiving the plaintext temporary password), weaken `dashboard_access` from `tailscale`/`lan` to `public`, or redirect `dashboard_domain`.
  - **Incomplete incident recovery:** Running `rig-server reset-password <email>` or admin `reset-password` after suspected compromise leaves the user's API tokens active.
- **Smallest Effective Fix:**
  1. Add a shared `SessionAuth<R>` extractor (or `session_only` check) requiring `Via::Session` on `/api/v1/tokens` (`POST`, `DELETE`), `/api/v1/members/*`, `/api/v1/system/dashboard-access`, `/api/v1/system/dashboard-domain`, `/api/v1/github/manifest*`, `/api/v1/github` (`DELETE`), and `/api/v1/alerts/*`.
  2. Cap active tokens per user (e.g., 25).
  3. Delete `api_tokens` for the target user inside admin `reset_password` and console `rig-server reset-password` transactions.

---

#### RIG-SEC-04: Unrate-Limited Password Verification Oracle on TOTP Management Endpoints
- **Category:** Missing Rate Limiting / Authentication Brute Force
- **Trust Boundary:** Hijacked browser session or unlocked browser tab → offline-speed online brute force of user password + Argon2id CPU DoS
- **Locations:**
  - `crates/rig-server/src/routes/account.rs:157-170` (`verify_password`), called by `:298` (`totp_setup`), `:383` (`totp_disable`), `:408` (`totp_regenerate`)
- **Root Cause:** Unlike `change_password` (`routes/members.rs:346-359`), `routes/account.rs::verify_password()` never calls `auth::check_rate` / `auth::record_failure`, never audits failed confirmations, and executes before `guarded_check` in `totp_disable` and `totp_regenerate`.
- **Impact:** An attacker with a stolen session cookie can submit unlimited password guesses without lockout or audit trail and can saturate server CPU with concurrent Argon2id verifications.
- **Smallest Effective Fix:** Enforce the existing `password:{user_id}` rate limit (`MAX_PASSWORD_FAILURES = 8` per 15 min) inside `verify_password()`, clear failures on success, and audit failed password confirmations.

---

#### RIG-SEC-05: Weakly Hashed TOTP Recovery Codes & Plaintext Webhook/Alert Secrets in SQLite
- **Category:** Cryptographic Storage Weakness / Secrets at Rest
- **Trust Boundary:** Disclosure of `rig.db` alone (e.g., via DB backup or `rig.db.pre-upgrade-*`, without `master.key`) → 2FA bypass, unauthorized deploy triggers, forged alert signatures
- **Locations:**
  - `crates/rig-server/src/totp.rs:120-133` (8-char base-31 recovery codes ≈ 39.6 bits; 10 active codes ≈ 36.3 bits effective)
  - `crates/rig-server/src/routes/account.rs:153-155` (`recovery_hash` uses single-round SHA-256)
  - `crates/rig-server/src/routes/apps.rs:548, 821-825` (`apps.webhook_secret` stored plaintext)
  - `crates/rig-server/src/routes/alerts.rs:113-125` (`alert_channels.signing_secret` stored plaintext alongside vault-encrypted `url_enc`)
- **Root Cause:** Recovery codes have low entropy relative to fast SHA-256 hashing (`8.5 × 10^11` keyspace is exhaustible on a single GPU in seconds). Separately, `apps.webhook_secret` and `alert_channels.signing_secret` were omitted from `Vault` encryption.
- **Impact:** If `rig.db` leaks without `master.key`, an attacker can rapidly crack a user's recovery code to bypass 2FA (defeating Vault encryption of `totp_secret`), trigger deployments on any app via its leaked `webhook_secret`, and forge signed alerts using `signing_secret`.
- **Smallest Effective Fix:**
  1. For recovery codes: either increase entropy to ≥ 128 bits (e.g., 16 random bytes formatted as `xxxx-xxxx-xxxx-xxxx-xxxx-xxxx` or hex) **or** hash recovery codes with Argon2id (or HMAC-SHA256 keyed by a subkey derived from `master.key`).
  2. Encrypt `apps.webhook_secret` and `alert_channels.signing_secret` via `state.vault.encrypt(...)`, with a startup/migration backfill that encrypts any legacy plaintext values.

---

#### RIG-SEC-06: Unauthenticated `/api/docs` Loads Unpinned Third-Party CDN JavaScript on Rig's Privileged Origin Without CSP
- **Category:** Unauthenticated Endpoint / Supply-Chain Same-Origin XSS
- **Trust Boundary:** Third-party CDN / npm package (`@scalar/api-reference@latest` on `cdn.jsdelivr.net`) → full same-origin JavaScript execution on Rig's authenticated dashboard origin
- **Locations:**
  - `crates/rig-server/src/routes/mod.rs:67-71` (`/api/openapi.json` and `/api/docs`)
  - `crates/rig-server/src/dashboard.rs:24-35, 79-86` (strict CSP applied only to fallback dashboard responses)
  - Dependency `utoipa-scalar-0.4.0/res/scalar.html:17`: `<script src="https://cdn.jsdelivr.net/npm/@scalar/api-reference"></script>`
- **Root Cause:** `/api/docs` and `/api/openapi.json` are registered directly on the router without authentication and bypass `dashboard::serve`'s `Content-Security-Policy` header. Scalar's default HTML loads an unversioned, non-SRI script from `cdn.jsdelivr.net`.
- **Impact:** Any compromise of `@scalar/api-reference` or `cdn.jsdelivr.net` executes arbitrary JavaScript in Rig's origin whenever an authenticated user visits `/api/docs`. Because `GET /api/v1/auth/me` returns the session's `csrf_token`, malicious JS can perform any authenticated Owner/Admin action (adding Owners, revealing all secrets, opening container shells). Additionally, unauthenticated actors can enumerate the entire API schema.
- **Smallest Effective Fix:**
  1. Gate `/api/openapi.json` and `/api/docs` behind `Auth<Viewer>` (or disable by default behind a `--enable-api-docs` flag, since `rig-server openapi` already generates the spec offline).
  2. If `/api/docs` remains enabled, self-host a pinned Scalar JS asset (or use `Scalar::custom_html` with an exact version, `integrity="sha384-..."`, `crossorigin="anonymous"`, and a strict CSP header).

---

#### RIG-SEC-07: Container-to-Host Proxy Access Breaks Project Isolation and `Access::Lan` Controls
- **Category:** Network Isolation Bypass
- **Trust Boundary:** Compromised app container in Project A → Project B's `Access::Lan` apps and Rig's `dashboard_access = lan` dashboard
- **Locations:**
  - `packaging/install.sh:658-661` (Caddy launched with `--network host`)
  - `crates/rig-server/src/proxy.rs:146-220` (Caddy listens on `:80`, `:443`, `:<port>` on all interfaces)
  - `crates/rig-server/src/access.rs:28-37` (`LAN_RANGES` includes `172.16.0.0/12` and `10.0.0.0/8`)
  - `crates/rig-server/src/deploy.rs:629-630` ("Projects cannot reach each other's containers")
- **Root Cause:** Caddy runs in the host network namespace and binds all interfaces (`0.0.0.0` / `::`), including every Docker (`172.17.0.0/16`, `172.18.0.0/16`, ...) and Podman (`10.88.0.0/16`, `10.89.0.0/16`, ...) bridge gateway. Connections from app containers to their bridge gateway reach Caddy with a source IP inside `172.16.0.0/12` or `10.0.0.0/8`, which matches `LAN_RANGES`.
- **Impact:** A compromised app container in Project A can reach Project B's internal `Access::Lan` apps via Caddy (violating inter-project isolation) and can reach the Rig dashboard even when `dashboard_access` is restricted to `lan`.
- **Smallest Effective Fix:** Exclude Docker/Podman bridge subnets from `LAN_RANGES` / Caddy's `remote_ip` allowlist (or add `not remote_ip` matchers for container bridge CIDRs), and document/configure host firewall rules blocking container bridge interfaces from reaching host ports `:80`, `:443`, `:7460`, and `:2019`.

---

### Medium Severity

#### RIG-SEC-08: Plaintext Redis Password in Container Command-Line Arguments
- **Locations:** `crates/rig-server/src/databases.rs:131-146`
- **Root Cause:** Redis starts with `["redis-server", "--appendonly", "yes", "--requirepass", password]`, contradicting the codebase's own invariant (`backups.rs:324`: `secrets_stay_off_the_command_line`).
- **Impact:** Exposes the 192-bit Redis password in host `/proc/<pid>/cmdline` (`ps aux` to any local OS user on default Linux `hidepid=0`) and in `docker inspect` / `podman inspect` `Config.Cmd`.
- **Fix:** Pass Redis configuration via a `0600` config/ACL file mounted into the container or stdin config (`redis-server -`) so the password never appears in argv.

---

#### RIG-SEC-09: Implicit State Machines, Incomplete Flows, and TOCTOU Races
Multiple state transitions rely on implicit checks or non-atomic multi-step sequences:

1. **Stateless 2FA login (`routes/auth.rs:264-345`, `web/src/routes/login.tsx:36, 61, 80-82`):**
   - No pre-auth challenge token exists; the browser stores the plaintext password in React state and re-transmits it with the TOTP code. Step 1 (`ApiError::SecondFactorRequired`) emits **no audit log entry**, allowing an attacker with a stolen password to confirm it works on a 2FA account silently.
   - *Fix:* Issue a short-lived (5-minute), single-use, server-side `pending_2fa` challenge token bound to IP/UA after password verification; audit step-1 password verification; verify the TOTP code against the challenge token without re-sending the password.
2. **Temporary passwords never expire or force change (`routes/members.rs:119-170, 290-323`, `main.rs:332-374`):**
   - There is no `must_change_password` column or expiry. Admin-issued temporary passwords remain valid permanently.
   - *Fix:* Add `must_change_password` and `temp_password_expires_at` on `users`, restricting authenticated sessions for flagged accounts to `POST /api/v1/auth/password` until updated.
3. **Session & TOTP lifecycle gaps (`routes/members.rs:369`, `routes/account.rs:253-260, 310, 367, 393`, `main.rs:295-330`):**
   - `change_password`, `totp_enable`, and `totp_disable` do not rotate the current session cookie/CSRF token (and filter other sessions by `csrf_token != ?` rather than `token_hash != ?`).
   - Console `rig-server reset2fa` revokes **zero** active sessions.
   - `totp_pending` has no expiry timestamp (abandoned enrollments stay valid forever in DB).
   - Sessions have a fixed 14-day TTL with no idle timeout and no user endpoint to view/revoke active sessions.
   - *Fix:* Rotate session + CSRF token on credential/2FA changes, revoke sessions on `reset2fa`, add `totp_pending_expires_at` (15 min), and expose a session-management endpoint.
4. **Destructive ordering bug in `delete_project` (`routes/projects.rs:261-293`):**
   - Lines 266–281 destroy **every app container and persistent app volume** in the project *before* line 282 checks whether any database is still `'creating'`. If a database is `'creating'`, the request fails with `409 Conflict` after already wiping the project's app data.
   - *Fix:* Move the `databases WHERE status = 'creating'` check before line 266.
5. **Non-atomic check-then-act (TOCTOU) races:**
   - **Deploy queueing** (`routes/apps.rs:761-782`, `routes/hooks.rs:118-147`, `github.rs:453-489`): Checking active deployments outside a transaction allows concurrent deploy/webhook requests to enqueue duplicate builds. *Fix:* Add a SQLite partial unique index `CREATE UNIQUE INDEX idx_deployments_active_app ON deployments(app_id) WHERE status IN ('queued','building','deploying')` (matching the atomic pattern already used in `create_backup`).
   - **Member role checks** (`routes/members.rs:181, 251, 295`): `load_member` + `check_can_manage` run outside the write transaction (racing with concurrent promotion to Admin/Owner). *Fix:* Perform the role check inside a `BEGIN IMMEDIATE` transaction.
   - **Database deletion & backup restore** (`routes/databases.rs:315-340, 579-609`): `delete_database` races with `start_database` (can leave orphaned containers); `restore_backup` has no lock preventing concurrent restores, backups, or stop/delete. *Fix:* Guard state transitions atomically via status updates (`'deleting'`, `'restoring'`) in SQLite.

---

#### RIG-SEC-10: Active Terminal WebSockets & SSE Log Streams Never Re-Check Auth/Revocation
- **Locations:** `crates/rig-server/src/routes/terminal.rs:57-147`, `crates/rig-server/src/routes/apps.rs:1005-1122`
- **Root Cause:** Authentication and role authorization occur only during the initial HTTP/WebSocket handshake. `session()` loops until `IDLE_TIMEOUT` (30 min) resets on every incoming frame, with no periodic re-validation and no cap on concurrent terminal sessions.
- **Impact:** A removed user, demoted Developer, or revoked session/API token retains live interactive shell access inside the app container indefinitely by sending periodic keystrokes.
- **Fix:** Pass the session/token hash and `(user_id, org_id)` into `session()` and re-verify validity + `Developer` role on a 30–60s timer inside `tokio::select!`, closing the WebSocket immediately if revoked/demoted. Enforce a cap on concurrent terminals per app/user (e.g., 5).

---

#### RIG-SEC-11: `check_origin` Ignores Scheme & Unconditionally Trusts `X-Forwarded-Host`; `real_ip` Reads First `X-Forwarded-For` Only
- **Locations:** `crates/rig-server/src/auth.rs:207-222, 238-249, 253-259`, `crates/rig-server/src/routes/terminal.rs:68-73`
- **Root Cause:**
  1. `check_origin` strips `scheme://` before comparing `Origin` to `Host`/`X-Forwarded-Host`, allowing `http://rig.example.com` to pass origin checks against an `https://rig.example.com` endpoint (critical for WebSocket terminal upgrades, which rely solely on `Origin` without a CSRF header).
  2. `check_origin` prefers `X-Forwarded-Host` and `is_https` trusts `X-Forwarded-Proto` without verifying `peer.is_loopback()`.
  3. `real_ip` uses `headers.get("x-forwarded-for")` (first header only) instead of `headers.get_all("x-forwarded-for")`.
- **Fix:** Verify the exact scheme (`https` when `is_https` is true, else `http`) plus host in `check_origin`; trust `X-Forwarded-*` headers only when `peer.is_loopback()` (or a configured trusted proxy CIDR); read the last entry across `headers.get_all("x-forwarded-for")`.

---

#### RIG-SEC-12: Git Clone & Poller Allow Internal Network Hosts, HTTP Redirects, and Broad Protocols
- **Locations:** `crates/rig-server/src/deploy.rs:690-704, 858-892`, `crates/rig-server/src/github.rs:625-643`
- **Root Cause:** `validate_git_url` permits `https://127.0.0.1:...`, `https://169.254.169.254/...`, `https://10.x.x.x/...`, and `ssh://127.0.0.1:...`. Git only denies `file` and `ext` (leaving other protocols and `http.followRedirects=initial` enabled, so an external `https://` repo can 302-redirect Git to an internal `http://` address).
- **Impact:** A `Developer` can initiate outbound connections from the host to internal/loopback/metadata services and inspect error messages streamed into deployment build logs.
- **Fix:** Use a strict Git protocol allowlist and disable redirects and symlinks:
  ```text
  -c protocol.allow=never
  -c protocol.https.allow=always
  -c protocol.ssh.allow=always
  -c http.followRedirects=false
  -c core.symlinks=false
  ```
  Validate resolved host IPs against private/loopback/link-local ranges before running `git clone` / `git ls-remote` (unless explicitly allowlisted for private Git servers).

---

#### RIG-SEC-13: Unpruned `login_attempts` & `sessions` Tables + Per-Email Account Lockout DoS
- **Locations:** `crates/rig-server/src/auth.rs:317-349`, `crates/rig-server/src/routes/auth.rs:274-314`, `crates/rig-server/migrations/0001_init.sql:37-47, 230-234`
- **Root Cause:** Rows in `login_attempts` are only deleted on a *successful* login (`clear_failures`), and expired `sessions` are never deleted. Simultaneously, `MAX_LOGIN_FAILURES_PER_EMAIL = 8` per 15 minutes globally locks out an email across all IPs.
- **Impact:**
  - Unauthenticated traffic sending failed logins for random emails/IPs grows `rig.db` unboundedly (eventually risking Rig's `< 1 GiB` free disk threshold that halts deploys and backups).
  - Any unauthenticated actor who knows an admin's email can permanently lock them out of the dashboard with 8 wrong passwords every 15 minutes.
- **Fix:** Add periodic cleanup (e.g., in the existing background metrics/maintenance loop) deleting `login_attempts WHERE ts < now - WINDOW_SECS` and `sessions WHERE expires_at < now`. Replace the hard 8-attempt per-email lockout with `(email, ip)` rate limiting plus progressive backoff / higher global email threshold with security alert notifications.

---

#### RIG-SEC-14: Missing Container `pids_limit`, Rootful Podman Privileged BuildKit, and Unencrypted DB Backups
- **Locations:** `crates/rig-server/src/runtime/docker.rs:127-138, 256, 415-440`, `crates/rig-server/src/backups.rs:218-248`
- **Root Cause:**
  1. `HostConfig` sets `memory` and `no-new-privileges`, but leaves `pids_limit: None`, allowing a fork bomb inside an app container to exhaust host kernel PIDs.
  2. `build_plan_buildkit` unconditionally sets `privileged: Some(true)` assuming rootless Podman, while `default_socket()` (`docker.rs:256`) and `--engine-socket` also accept rootful `/run/podman/podman.sock`.
  3. Database dumps under `<data_dir>/backups/` are stored plaintext (`0600`).
- **Fix:** Set a default `pids_limit` (e.g., `Some(512)`) and drop `NET_RAW`; verify Podman is running rootless before permitting `privileged: true` (or use rootless BuildKit); consider streaming AEAD encryption for backup files on disk.

---

### Low / Hardening

#### RIG-SEC-15: Missing HSTS Header & `Cache-Control: public` on Private Avatars
- **Locations:** `crates/rig-server/src/routes/mod.rs:79-90`, `crates/rig-server/src/routes/org.rs:351-390`
- **Issue:** Neither Rig nor Caddy emits `Strict-Transport-Security` over HTTPS. `get_media` returns `Cache-Control: public, max-age=31536000, immutable` even for private avatars (`is_public = 0`), and returns `401` for existing private IDs vs `404` for non-existent IDs.
- **Fix:** Add `Strict-Transport-Security: max-age=31536000; includeSubDomains` when serving over HTTPS; set `Cache-Control: private, max-age=31536000, immutable` when `is_public == 0`; return uniform `404` when unauthenticated or cross-org callers request private media.

#### RIG-SEC-16: Bypassable SVG Blocklist Sanitizer & External Image Tracking Beacons
- **Locations:** `crates/rig-server/src/routes/org.rs:44-59, 77-118`, `crates/rig-server/src/dashboard.rs:27`
- **Issue:** `is_safe_svg` relies on substring blacklisting that is bypassed via XML entities (`&#106;avascript:`), `<animate>`, `<set>`, `<use href="data:...">`, etc. (mitigated in modern browsers by `Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; sandbox` on `get_media` and `<img>` rendering in React). Separately, allowing arbitrary external `http://`/`https://` avatar/logo URLs (paired with `img-src ... https: http:`) lets any `Viewer` inject a tracking pixel loaded by Admins/Owners viewing the Team page.
- **Fix:** Restrict uploads to raster formats (PNG/JPEG/WebP/GIF) or sanitize SVGs with a strict XML allowlist; restrict external URLs to `https://` only or phase them out in favor of uploaded media/presets.

#### RIG-SEC-17: Operational & Defense-in-Depth Hygiene
- **Locations:** `crates/rig-server/src/main.rs:255-259`, `crates/rig-server/src/github.rs:383-399`, `crates/rig-server/src/routes/github.rs:264`, `crates/rig-server/src/routes/auth.rs:133`, `packaging/install.sh:655`
- **Items:**
  1. `main.rs:256` writes the one-time setup token to stderr (`eprintln!`), which `systemd` captures into `journald` (`journalctl -u rig`) contrary to the comment; print only when stderr is a TTY, relying on `rig-server setup-token` otherwise.
  2. Cap the in-memory `github::STATES` map (e.g., max 64 entries) or persist pending states in SQLite with TTL pruning.
  3. Restrict `POST /api/v1/hooks/github/{org_id}` to HMAC `X-Hub-Signature-256` only rather than also accepting plain `X-Gitlab-Token`.
  4. Omit `version` from unauthenticated `GET /api/v1/setup`.
  5. In `packaging/install.sh:655`, mount `/run/tailscale` into the Caddy container only when Tailscale TLS is configured, rather than whenever the host directory exists.

---

## 6. Pre-Release Remediation Checklist

### Phase 0 — Must-Fix Before Release (High)
- [ ] **RIG-SEC-06:** Authenticate `/api/docs` & `/api/openapi.json` (or gate behind `--enable-api-docs`) and eliminate unpinned `cdn.jsdelivr.net` script loading on Rig's origin.
- [ ] **RIG-SEC-01:** Add connect-time IP validation (custom DNS resolver) to `alerts.rs` blocking loopback, link-local, private, and CGNAT ranges; sanitize `test_channel` errors.
- [ ] **RIG-SEC-02 & RIG-SEC-12:** Harden `git clone` / `git ls-remote` with `-c protocol.allow=never -c protocol.https.allow=always -c protocol.ssh.allow=always -c http.followRedirects=false -c core.symlinks=false`, validate resolved Git hosts, and reject symlinks escaping `checkout` before `railpack prepare`.
- [ ] **RIG-SEC-03:** Enforce `Via::Session` on token creation/deletion, member management, dashboard access/domain changes, GitHub app setup, and alert channels; cap tokens per user; revoke `api_tokens` on admin and console password resets.
- [ ] **RIG-SEC-04:** Rate-limit and audit `routes/account.rs::verify_password()`.
- [ ] **RIG-SEC-05:** Encrypt `apps.webhook_secret` and `alert_channels.signing_secret` with `Vault`; upgrade TOTP recovery codes to ≥ 128-bit entropy or Argon2id / Vault-keyed HMAC-SHA256.
- [ ] **RIG-SEC-07:** Exclude Docker/Podman bridge subnets from `LAN_RANGES` / Caddy `remote_ip` allowlists and isolate host Caddy/Rig ports from container bridges.

### Phase 1 — Strong Pre-Release Hardening (Medium)
- [ ] **RIG-SEC-08:** Move Redis `--requirepass` off container argv.
- [ ] **RIG-SEC-09d/e:** Fix `delete_project` check ordering; add partial unique index for active deployments; move member role checks inside write transactions; add atomic state locks for `delete_database` and `restore_backup`.
- [ ] **RIG-SEC-10:** Periodically re-verify session/token and role inside active terminal WebSockets and SSE log streams; add concurrent terminal caps.
- [ ] **RIG-SEC-11:** Enforce scheme matching in `check_origin`; trust `X-Forwarded-*` only from loopback/trusted proxies; read all `X-Forwarded-For` headers.
- [ ] **RIG-SEC-13:** Prune expired `login_attempts` and `sessions` periodically; replace global 8-attempt email lockout with `(email, ip)` throttling / backoff.
- [ ] **RIG-SEC-14:** Set default container `pids_limit` and drop `NET_RAW`; verify rootless Podman before privileged BuildKit; consider encrypting DB backups at rest.

### Phase 2 — Auth Flow & Defense-in-Depth Polish (Medium/Low)
- [ ] **RIG-SEC-09a:** Implement short-lived server-side 2FA challenge token for step-2 login and audit step-1 password verification.
- [ ] **RIG-SEC-09b:** Add `must_change_password` and expiry for admin-issued temporary passwords.
- [ ] **RIG-SEC-09c:** Rotate session/CSRF token on password and 2FA changes; revoke sessions on console `reset2fa`; add `totp_pending` TTL; expose active session list/revoke API.
- [ ] **RIG-SEC-15/16/17:** Add HSTS on HTTPS, `Cache-Control: private` on private media, harden/remove SVG upload & external avatar URLs, keep setup token out of `journald`, cap `github::STATES`, tighten GitHub webhook verification, and scope `/run/tailscale` mount.
