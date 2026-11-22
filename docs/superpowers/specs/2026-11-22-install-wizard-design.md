# Rig Interactive Install Wizard, HTTPS First-Run & CLI Password Reset

Date: 2026-11-22
Status: Implemented

---

## 1. Intent & Why

Today, installing Rig via `packaging/install.sh` is completely non-interactive and always boots the dashboard on plain HTTP (`http://<server-ip>`). That creates three problems for users:

1. **Unencrypted first-time setup:** The owner's initial password is submitted over plain HTTP unless they manually configure a domain after setup.
2. **No guided choices or discoverability:** Users cannot choose Docker vs. rootless Podman, attach a dashboard domain, enable HTTPS, or restrict access (Public / LAN / Tailscale) during installation, nor pass CLI flags for scripted installs.
3. **Distribution & recovery gaps:** Users have to manually download and unpack a tarball instead of running `curl -fsSL ... | sudo bash`, and there is no server-console command to reset a forgotten owner/user password (unlike `rig-server reset2fa`).

The goal is a fast, polished, auditable installation experience that secures first-time browser setup with HTTPS from minute one, supports both interactive arrow-key TUI and flag-driven unattended installs, and provides a clean CLI password-reset escape hatch.

---

## 2. Goals, Non-Goals & Anti-Goals

### Goals
- **Two launch paths, one installer:** Support both `curl -fsSL <github-release-url>/install.sh | sudo bash` (with optional flags via `bash -s -- ...`) and local `sudo ./install.sh` from an unpacked tarball.
- **Polished interactive TUI wizard:** Arrow-key selection and clear inline status labels (installed, will install, unavailable + reason) for container engine, install mode, dashboard domain/HTTPS, and access scope.
- **HTTPS before first browser login:** Allow configuring a custom domain (Let's Encrypt `auto` with live DNS check), Tailscale HTTPS (`tailscale`, when Tailscale is installed), or self-signed HTTPS (`internal` on IP/LAN hostname), setting `RIG_SECURE_COOKIES=true` when HTTPS is active.
- **First-class CLI flags & non-interactive parity:** Support `--podman`, `--docker`, `--engine`, `--domain`, `--tls`, `--access`, `--yes`/`-y`, `--version`, `--repo`, `--uninstall`, `--help`, plus existing `RIG_*` env vars. Unattended runs (`--yes` or no `/dev/tty`, e.g., `scripts/vm-test.sh` and CI) and in-place upgrades never hang waiting for input.
- **Server-console password reset:** Add `rig-server reset-password <email>` to generate a strong random password, revoke all active sessions for that user, clear login rate-limit entries, log an audit event, and print the password once.

### Non-Goals
- Implementing headless mode itself (the wizard shows it as "coming soon" and rejects `--headless` cleanly until headless support lands in a separate change).
- Installing Tailscale automatically (Tailscale options are only enabled when Tailscale is already installed on the host).
- Adding extra system administration steps to the wizard (no UFW/firewall mutation, no swap creation, no custom port/data-dir prompts, no interactive post-install management menu).
- Moving owner account creation into the terminal (first-time owner creation stays in the browser via the 30-minute one-time setup token).

### Anti-Goals (failure modes to avoid)
- **Locking out the user on first boot:** Enabling `Tailscale only` access when Tailscale is disconnected, or committing a domain with broken DNS without warning and offering retry/fallback.
- **Breaking piped installs:** Reading prompts from stdin instead of `/dev/tty` when run via `curl ... | sudo bash`.
- **Hiding root mutations in a binary:** Putting `apt`/`dnf`/`useradd`/`systemctl` inside compiled Rust where sysadmins cannot inspect it before `sudo`.
- **Breaking upgrades or `scripts/vm-test.sh`:** Prompting during upgrades or non-TTY CI runs, or changing the recorded engine on an existing install.
- **Root-owned files in `/var/lib/rig`:** Running `rig-server install-wizard` as root must never create or touch `/var/lib/rig`, `rig.db`, or `master.key`.

---

## 3. Constraints

- **Supported platforms:** Debian 12+, Ubuntu 24.04+, Fedora 40+ / RHEL 9+ (`x86_64`), systemd required, root (`sudo`) required.
- **Podman minimum version:** Podman 4.9+ (Debian 12 ships 4.3, so Podman must be marked unavailable on Debian 12 unless a 4.9+ binary is already installed).
- **RHEL/CentOS Docker:** Default RHEL 9+ repos do not include `docker.io` / `moby-engine`; Docker is only selectable there if already installed.
- **No extra host dependencies:** `install.sh` cannot assume `jq`, `dig`, or `nc` are installed on a fresh minimal server.
- **Binary size & crate restraint:** Rig compiles with `opt-level = "z"`, `lto = "thin"`. The TUI prompt library added to `rig-server` should be small and well-maintained (`cliclack`).
- **Release distribution:** Default GitHub Releases repository is `ksaurav24/rig` (matching `git remote origin`), overridable via `--repo` or `RIG_REPO`. Checksum verification via `.sha256` is mandatory for downloaded tarballs.

---

## 4. Approaches Considered

1. **Self-contained pure Bash wizard (`install.sh` only) + `rig-server reset-password`:**
   Zero new Rust dependencies and prompts before download, but limited to numbered prompts (no arrow-key TUI) and fragile shell JSON/DNS parsing.
2. **Thin Bash bootstrapper + full Rust installer inside `rig-server`:**
   Slick arrow-key TUI and strong Rust typing, but hides root system commands (`apt`, `dnf`, `useradd`, `systemctl`) inside a compiled binary, hurting auditability for `curl | sudo bash`.
3. **Hybrid — Bash handles bootstrap & all root system execution; `rig-server install-wizard` handles only the interactive TUI prompts and probing *(Chosen)*:**
   Combines a polished arrow-key TUI, typed DNS/Tailscale/host validation, and full code reuse (`validate_host`, `TlsMode`, `Access`) with a 100% readable, auditable `install.sh` for every system change. Trade-off: in `curl | sudo bash` mode, the release tarball is downloaded and verified before the interactive menu appears (acceptable for a ~few-second download).

---

## 5. Detailed Design

### 5.1 Entry Points & GitHub Release Bootstrap (`packaging/install.sh`, `scripts/package.sh`)

One script, `packaging/install.sh`, serves both local-bundle and remote `curl | sudo bash` runs:

1. **Argument & env parsing (runs first):**
   `install.sh` parses CLI flags before doing any network or system work:

   | Flag | Env var | Meaning |
   | --- | --- | --- |
   | `--podman` | `RIG_ENGINE=podman` | Select rootless Podman |
   | `--docker` | `RIG_ENGINE=docker` | Select Docker |
   | `--engine <podman\|docker>` | `RIG_ENGINE` | Explicit engine choice |
   | `--mode <full\|headless>` | `RIG_MODE` | Install mode (`full` default; `headless` exits with a clear "coming soon" message) |
   | `--headless` | — | Shorthand for `--mode headless` |
   | `--domain <host>` | `RIG_DOMAIN` | Dashboard domain or IP for Caddy |
   | `--tls <auto\|internal\|tailscale\|off>` | `RIG_TLS` | Dashboard TLS mode |
   | `--access <public\|lan\|tailscale>` | `RIG_ACCESS` | Dashboard network access scope |
   | `-y`, `--yes`, `--non-interactive` | `RIG_YES=1` | Skip interactive prompts; use flags/env + safe defaults |
   | `--version <tag>` | `RIG_VERSION` | Release version to download (e.g. `0.1.0` or `v0.1.0`; default: latest) |
   | `--repo <owner/repo>` | `RIG_REPO` | GitHub repository for releases (default: `ksaurav24/rig`) |
   | `--uninstall` | — | Stop and remove Rig (keeps `/var/lib/rig` and `/etc/rig`) |
   | `-h`, `--help` | — | Print usage and exit 0 |

   - Conflicting flags (e.g., `--podman --docker`, or `--tls auto` with an IP literal) fail immediately with a clear error message.
   - `RIG_PUBLIC_ADDRESS` and `RIG_CADDY_IMAGE` continue to work as before for backward compatibility (e.g., `scripts/vm-test.sh`).

2. **Bundle resolution:**
   - Check if `$HERE/rig-server`, `$HERE/rig`, and `$HERE/rig.service` exist alongside `install.sh` (only when `BASH_SOURCE[0]` is a real file, not stdin).
   - If not (i.e., `curl -fsSL ... | sudo bash`), enter **release download mode**:
     - Ensure `curl` and `tar` are installed (installing `curl ca-certificates` via distro packages if missing).
     - Determine target tag:
       - If `--version` is given, normalize `tag="v${ver#v}"` and `ver="${ver#v}"`.
       - Otherwise resolve latest tag without hitting the rate-limited GitHub REST API by following the redirect on `https://github.com/$REPO/releases/latest` (`curl -fsSL -o /dev/null -w '%{url_effective}'`) and extracting the trailing tag segment.
     - Download `rig-${ver}-x86_64-linux.tar.gz` and `rig-${ver}-x86_64-linux.tar.gz.sha256` from `https://github.com/$REPO/releases/download/$tag/` into a temporary directory (`mktemp -d`, removed on `EXIT` trap).
     - Verify `sha256sum -c` (aborting on mismatch).
     - Unpack tarball and point `HERE` to the unpacked bundle directory.
   - `scripts/package.sh` will also copy `packaging/install.sh` to `dist/install.sh` (chmod `0755`) so release automation can upload `install.sh` directly to GitHub Releases alongside the tarball and `.sha256`.

### 5.2 Upgrade vs. Fresh Install & Non-Interactive Rules

- **Existing install detection:** An install is treated as an **upgrade** when `/etc/rig/rig.env` already exists.
  - On upgrade, the interactive wizard is **skipped**: the recorded `RIG_ENGINE` is preserved (rejecting any conflicting `--engine`/`--podman`/`--docker`), `/etc/rig/rig.env` is preserved, `rig.db` is backed up before replacing binaries, and existing DB domain/access settings stay untouched.
  - If the user passes `--domain`, `--tls`, or `--access` explicitly during an upgrade, warn and ignore (or clearly tell them to use `rig-server dashboard-domain` / `rig-server dashboard-access` on an existing install — **decision: fail fast with a message pointing to `rig-server dashboard-domain` / `rig-server dashboard-access` so flags are never silently ignored**).
- **Non-interactive mode:** Triggered when `-y`/`--yes`/`--non-interactive` is set, OR `/dev/tty` is not readable/writable (such as in `scripts/vm-test.sh` or cloud-init), OR all configurable choices were already provided via CLI flags/env vars.
  - Defaults in non-interactive mode:
    - Engine: existing Docker > existing Podman >= 4.9 > installable Podman >= 4.9 > installable Docker (exact current `install.sh` heuristic).
    - Mode: `full`.
    - TLS/Domain:
      - If `--domain` is set and `--tls` is omitted → `--tls auto` (unless `--domain` is an IP literal, in which case error and suggest `--tls internal` or `--tls off`).
      - If `--tls tailscale` and `--domain` is omitted → auto-detect Tailscale MagicDNS name (fail if Tailscale is not running or has no `DNSName`).
      - If `--tls internal` and `--domain` is omitted → use detected server IP (or `RIG_PUBLIC_ADDRESS`).
      - If neither `--domain` nor `--tls` is set → plain HTTP (`--tls off`, `http://<addr>`), preserving full backward compatibility with `scripts/vm-test.sh` and automated scripts.
    - Access: `--access` if set, else `tailscale` when `--tls tailscale`, else `public`.

### 5.3 System Probing & Bash ↔ Rust Handoff (`rig-server install-wizard`)

Before invoking the wizard on a fresh interactive install, `install.sh` collects lightweight distro facts that depend on the host package manager and passes them to `rig-server install-wizard`:

```sh
plan_file=$(mktemp)
"$HERE/rig-server" install-wizard \
  --output "$plan_file" \
  --distro-name "$PRETTY_NAME" \
  --ram-mb "$mem_mb" \
  --disk-gb "$disk_gb" \
  --docker-installed "$docker_ver" \       # empty if not installed
  --docker-installable "$docker_can" \     # true|false (false on RHEL/CentOS without repo)
  --podman-installed "$podman_ver" \       # empty if not installed
  --podman-candidate "$podman_cand" \      # distro repo candidate version, or empty
  ${OPT_ENGINE:+--engine "$OPT_ENGINE"} \
  ${OPT_DOMAIN:+--domain "$OPT_DOMAIN"} \
  ${OPT_TLS:+--tls "$OPT_TLS"} \
  ${OPT_ACCESS:+--access "$OPT_ACCESS"} \
  </dev/tty
```

**Critical boundary rules:**
- `Command::InstallWizard` returns early in `crates/rig-server/src/main.rs` right alongside `Command::Openapi` — **before** creating `cli.data_dir`, opening SQLite, or opening `Vault`. It never writes to `/var/lib/rig`.
- Interactive TUI renders to `/dev/tty` (stdout/stderr of the terminal) using `cliclack`.
- The wizard writes the final choices to `--output "$plan_file"` using single-quoted shell assignments with safe escaping (only validated values: `PLAN_ENGINE`, `PLAN_MODE`, `PLAN_DOMAIN`, `PLAN_TLS`, `PLAN_ACCESS`, `PLAN_PUBLIC_URL`, `PLAN_SECURE_COOKIES`).
- If the user cancels (`Ctrl+C` or declines the final confirmation), `install-wizard` exits non-zero (exit code `130` for cancel) and `install.sh` exits cleanly without making any system changes.

### 5.4 Interactive Wizard UX Flow (`crates/rig-server/src/wizard.rs`)

Inside `rig-server install-wizard`:

1. **Header & live network probe:**
   - Clears the terminal (`cliclack::clear_screen()`) for readability, then displays `intro("Rig Installer v<version>")` and a compact system summary line: OS, arch, RAM, free disk (highlighting warnings if RAM < 900 MB or disk < 5 GB).
   - Runs fast concurrent probes (`tokio::join!`):
     - **Tailscale probe:** reuses the `tailscale status --json --peers=false` check (3s timeout) plus checks `tailscale` presence on `PATH`. Produces one of three states: `Connected { dns_name, ips }`, `InstalledNotConnected`, or `NotInstalled`.
     - **IP probe:** detects local interface IPv4 (via UDP socket trick or `sysinfo::Networks`) AND attempts a 2-second external public IPv4 lookup (`https://api.ipify.org`, fallback `https://ifconfig.me/ip`), so DNS checks work even on NATed cloud VMs (AWS/GCP/Oracle). Falls back to interface IP if offline.

2. **Step 1 — Container engine** *(skipped if `--engine` / `--podman` / `--docker` was already passed)*:
   - Evaluates Podman availability (`installed >= 4.9`, `installed < 4.9` with `candidate >= 4.9` auto-upgrade, or `candidate >= 4.9`) and Docker availability (`installed` or `installable`).
   - If neither engine is available on this distro, prints an actionable error and exits 1 before prompting.
   - Always keeps both options visible in the arrow-key select with inline status tags:
     - **Default selection:** Already-installed engine (if exactly one is installed, or Docker if both are installed); if neither is installed, rootless Podman when >= 4.9 is available, else Docker.
     - **Older Podman upgrade:** If Podman `< 4.9` is installed and the distro ships `4.9+`, Podman stays selectable (`will upgrade to vX.Y`) and `install.sh` upgrades it automatically.
     - **Unavailable option handling:** If Podman cannot reach `4.9+` (e.g., Debian 12 ships 4.3) or Docker is not installable (e.g., RHEL without repo), keep the item visible with `[upgrade needed]` / `[not installed]`; selecting it displays actionable install/upgrade instructions and re-prompts.

3. **Step 2 — Install mode:**
   - Arrow-key select:
     - `Full (API + web dashboard)` — default.
     - `Headless (API & CLI only) [coming soon]` — hint: `not available yet`.
   - If the user picks `Headless`, show a friendly note that headless mode is coming in a future release and re-prompt so they can pick `Full` or cancel.

4. **Step 3 — Dashboard address & HTTPS** *(skipped if `--domain` or `--tls` was already passed)*:
   - Arrow-key select:
     1. **Custom domain with automatic HTTPS (Let's Encrypt)** *(recommended for public servers)*
        - Prompts for hostname (e.g., `rig.example.com`). Validates via `routes::apps::validate_host` AND rejects bare IPv4/IPv6 literals (Let's Encrypt requires a DNS name).
        - Runs a live DNS lookup (`tokio::net::lookup_host((host, 80))`) with a spinner and checks whether any resolved IP matches this server's detected public or LAN IP:
          - **Matches:** logs `✓ <host> resolves to <ip> (this server)` and moves on.
          - **Does not resolve or resolves to a different IP:** warns clearly (showing expected server IP vs. what resolved) and offers:
            - `Re-check DNS` (lets user add/fix the A record in another tab and retry in place)
            - `Continue anyway` (proceeds; notes `rig-server dashboard-domain --clear` escape hatch)
            - `Choose a different address/HTTPS option` (returns to Step 3 menu)
     2. **Tailscale HTTPS** *(always shown; only selectable when Tailscale is `Connected` with a `dns_name`)*
        - If `Connected` with `dns_name`: selectable as `Tailscale HTTPS (<name>.ts.net)`.
        - If `Connected` without MagicDNS, `InstalledNotConnected`, or `NotInstalled`: kept visible with `[MagicDNS off]`, `[not connected]`, or `[not installed]`; selecting it explains what to install/enable first and re-prompts.
     3. **Self-signed HTTPS (Private CA)** *(encrypted setup without a domain)*
        - Prompts for host/IP, pre-filled with the server's detected public (or LAN) IP. Validates with `validate_host`.
        - Notes that the browser will display a one-time certificate warning, while keeping the setup token and owner password encrypted in transit.
     4. **Plain HTTP** *(not recommended)*
        - Shows a clear warning that the first-time password will travel unencrypted unless accessed via an SSH tunnel (`ssh -L 7460:127.0.0.1:7460 <user>@<ip>`).

5. **Step 4 — Dashboard access scope** *(skipped if `--access` was already passed)*:
   - Arrow-key select:
     - `Public` — anyone who can reach this server.
     - `LAN only` — private network ranges only.
     - `Tailscale only` — always visible; selectable when Tailscale is `Connected`, otherwise tagged `[not installed]` / `[not connected]` and re-prompts with guidance if selected (preventing immediate lockout).
   - Smart default: `Tailscale only` when Tailscale HTTPS was chosen; `Public` otherwise (or `LAN only` if the user entered a private RFC1918 IP for self-signed HTTPS).

6. **Step 5 — Review summary & confirm:**
   - Displays a clean boxed summary of every setting:
     - Engine (`Podman (rootless)` or `Docker`, plus whether it will be installed)
     - Mode (`Full`)
     - Dashboard URL (`https://rig.example.com`, `https://box.tail.ts.net`, `https://203.0.113.10`, or `http://203.0.113.10`)
     - TLS (`Automatic (Let's Encrypt)` / `Tailscale` / `Self-signed (Private CA)` / `Off`)
     - Access (`Public` / `LAN only` / `Tailscale only`)
   - `Confirm & install?` (`Yes` default). On `Yes`, writes `--output "$plan_file"` and exits 0.

### 5.5 Applying the Plan in `packaging/install.sh`

Once the plan is resolved (from either `install-wizard` or non-interactive flags/defaults):

1. Installs missing `git`/engine packages, sets up `rig` user, rootless Podman or Docker group, checks ports `80`/`443`, installs `rig-server` and `rig` binaries.
2. **Tailscale + rootless Podman permission:** When `PLAN_TLS=tailscale` and `ENGINE=podman`:
   - Configures Tailscale to permit the `rig` user's UID to request TLS certificates (via `tailscale set --operator=rig` if supported, and/or checking `/run/tailscale/tailscaled.sock` permissions + printing a clear note if `TS_PERMIT_CERT_UID=$(id -u rig)` is needed on their Tailscale version, plus reminding them that HTTPS Certificates must be enabled in the Tailscale admin console).
3. **Environment file (`/etc/rig/rig.env`):**
   - Writes `RIG_PUBLIC_URL` (`https://<domain>` when TLS is enabled, else `http://<addr>`).
   - Writes `RIG_SECURE_COOKIES=true` when `PLAN_TLS` is `auto`, `internal`, or `tailscale` (commented out when `off`).
   - Writes `RIG_ENGINE` and `RIG_ENGINE_SOCKET`.
4. **Database pre-configuration before starting `rig.service`:**
   - If `PLAN_DOMAIN` is non-empty:
     `runuser -u rig -- "$BIN_DIR/rig-server" --data-dir "$DATA_DIR" dashboard-domain "$PLAN_DOMAIN" --tls "$PLAN_TLS"`
   - If `PLAN_ACCESS` is `lan` or `tailscale`:
     `runuser -u rig -- "$BIN_DIR/rig-server" --data-dir "$DATA_DIR" dashboard-access "$PLAN_ACCESS"`
5. Starts `rig-caddy` and `rig.service`, waits for `http://127.0.0.1:7460/api/v1/setup`.
   - When `PLAN_TLS != off`, also checks/waits briefly for Caddy to accept the config from Rig's proxy supervisor so the HTTPS endpoint is live before printing the setup link.
6. Prints the final screen:
   - The `https://...` (or `http://...`) URL and 30-minute one-time setup token.
   - If `PLAN_TLS=off`, prints the SSH tunnel alternative (`ssh -L 7460:127.0.0.1:7460 ...`).
   - Compact day-to-day & recovery cheat sheet:
     - New setup token: `sudo -u rig rig-server --data-dir /var/lib/rig setup-token`
     - Reset password: `sudo -u rig rig-server --data-dir /var/lib/rig reset-password <email>`
     - Clear dashboard domain: `sudo -u rig rig-server --data-dir /var/lib/rig dashboard-domain --clear`
     - Reset dashboard access: `sudo -u rig rig-server --data-dir /var/lib/rig dashboard-access public`
     - Logs: `journalctl -u rig -f`

### 5.6 Server-Side Changes (`crates/rig-server`)

1. **`rig-server reset-password <email>`** (in `crates/rig-server/src/main.rs`):
   - Subcommand `ResetPassword { email: String }`, mirroring `Reset2fa { email: String }`.
   - Trims and lowercases `email`, looks up `users.id` (bails with `"no user with the email {email}"` if not found).
   - Generates a random strong password via `crypto::random_token(15)` (matching `routes/members.rs:297`).
   - Hashes with `crypto::hash_password(&password)`.
   - Inside a single SQLite transaction:
     - Updates `users.password_hash` for that user.
     - Deletes all rows from `sessions WHERE user_id = ?` (revokes all existing browser sessions).
     - Deletes login rate-limit rows for `login:email:<email>` from `login_attempts` (so a locked-out user can sign in immediately).
     - Inserts an `audit_log` entry (`actor = 'console'`, `action = 'auth.password_reset'`, `target = email`, `detail = 'reset from the server console'`).
   - Prints the new password once to stdout and reminds the user to sign in and change it in the dashboard.
2. **Bare-IP & TLS validation check:**
   - Verify `proxy::caddy_config` + `dashboard-domain` works end-to-end when `host` is an IPv4 address with `TlsMode::Internal` (and reject bare IP addresses when `tls == "auto"` in `dashboard-domain` and `install-wizard`, since Let's Encrypt HTTP-01 requires a domain name).
3. **`rig-server install-wizard`** (new module `crates/rig-server/src/wizard.rs`, crate `cliclack`):
   - Handles all interactive prompts, Tailscale detection, public/LAN IP detection, DNS resolution checks, validation, and shell-safe plan file output.
   - Early-exits in `main.rs` before `data_dir` creation or DB/Vault initialization.

---

## 6. Verification Plan (What Must Be Tested)

- **Rust unit tests:**
  - `reset-password` updates hash, revokes sessions, clears email rate-limit failures, writes audit log, and fails cleanly on unknown email.
  - `wizard` helpers: version comparison (`>= 4.9`), host/IP classification (`auto` rejects IPs, `internal` allows IPs), Tailscale JSON parsing across all 3 states, shell plan serialization/escaping, default engine/access selection logic.
  - `proxy::caddy_config` test for dashboard domain on bare IPv4 with `TlsMode::Internal`.
- **VM integration tests (`scripts/vm-test.sh`):**
  - Existing non-interactive Docker & Podman flows remain green (verifying no regression).
  - Non-interactive flag installs: `--docker`/`--podman`, `--domain rig.vm.test --tls internal --access public` (verifying HTTPS works via `curl -k https://...` and session cookie is marked `Secure`), and `--tls internal` on bare IP.
  - CLI password reset test: run `rig-server reset-password owner@example.com`, verify old session is invalidated, verify login succeeds with the printed random password and fails with the old password.
- **Manual interactive smoke test:**
  - Test both `sudo ./install.sh` and simulated `cat install.sh | sudo bash` (confirming `/dev/tty` arrow-key TUI works cleanly through a pipe, including Ctrl+C cancel and DNS retry loop).

---

## 7. Left to the Builder

- Exact `cliclack` styling/wording, spinner phrasing, and color formatting in `wizard.rs`.
- Structuring helper functions inside `packaging/install.sh` and exact temp-file cleanup traps.
- Extracting shared Tailscale/LAN probing helpers from `routes/system.rs` so `wizard.rs` and `routes/system.rs` do not duplicate `tailscale status --json` parsing.
- Updating `docs/deploy-on-a-server.md` and `README.md` to document `curl | sudo bash`, CLI flags, HTTPS during install, and `rig-server reset-password`.
