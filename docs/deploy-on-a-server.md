# Running Rig on a server

This puts Rig on a fresh Linux VM (any cloud, or a box at home) and deploys a
first app. It takes about five minutes.

## What you need

- A 64-bit x86 VM running Debian 12+, Ubuntu 24.04+ or Fedora 40+ / RHEL 9+,
  with 1 GB of memory or more and 10 GB of free disk.
- Root access (`sudo`).
- Ports **80** and **443** open in the provider's firewall. Nothing else
  needs to be exposed: Rig itself listens on `127.0.0.1:7460` and is reached
  through its proxy.
- Optional: a domain whose DNS you control, for HTTPS.

## 1. Install on your server (recommended)

Run the interactive installer directly from GitHub Releases:

```sh
curl -fsSL https://github.com/ksaurav24/rig/releases/latest/download/install.sh | sudo bash
```

Or pass flags directly (for scripted installs or to skip prompts):

```sh
curl -fsSL https://github.com/ksaurav24/rig/releases/latest/download/install.sh \
  | sudo bash -s -- --podman --domain rig.example.com --tls auto --access public
```

### Or build a bundle yourself (from source)

```sh
scripts/package.sh
# -> dist/rig-0.1.0-x86_64-linux.tar.gz and dist/install.sh
scp dist/rig-0.1.0-x86_64-linux.tar.gz you@your-vm:
ssh you@your-vm
tar xzf rig-0.1.0-x86_64-linux.tar.gz && cd rig-0.1.0-x86_64-linux
sudo ./install.sh
```

The interactive wizard clears the terminal and walks you through:

- **Container engine:** rootless Podman (4.9+) or Docker. Missing packages are installed from your distro automatically, existing Docker installs are detected and pre-selected, older installed Podman (`< 4.9`) is upgraded when your distro ships `4.9+`, and unavailable options stay visible with clear guidance (e.g. `[upgrade needed]` on Debian 12's Podman 4.3).
- **Mode:** Full (API + web dashboard; `Headless [coming soon]` is listed for visibility).
- **Dashboard address & HTTPS:** configure HTTPS right during installation so your initial password never crosses the network unencrypted:
  - **Custom domain + Let's Encrypt (`--domain rig.example.com --tls auto`)** with live DNS verification and retry;
  - **Tailscale HTTPS (`--tls tailscale`)** when Tailscale is installed and connected (shown as `[not installed]` or `[not connected]` otherwise);
  - **Self-signed HTTPS (`--tls internal`)** on your server IP or LAN hostname; or
  - **Plain HTTP (`--tls off`)** with an SSH tunnel tip.
- **Dashboard access scope:** Public, LAN only, or Tailscale only (`--access public|lan|tailscale`).

### Installer flags

All choices can also be passed on the command line (or via `RIG_*` environment variables). Partial flags skip those prompts in the wizard; pass `-y` / `--yes` (or run without a TTY) for fully unattended installs.

| Flag | Env var | Description |
| --- | --- | --- |
| `--podman` / `--docker` | `RIG_ENGINE` | Select rootless Podman or Docker |
| `--engine <podman\|docker>` | `RIG_ENGINE` | Explicit container engine |
| `--domain <host>` | `RIG_DOMAIN` | Dashboard domain or IP |
| `--tls <auto\|internal\|tailscale\|off>` | `RIG_TLS` | Dashboard HTTPS mode |
| `--access <public\|lan\|tailscale>` | `RIG_ACCESS` | Who may reach the dashboard |
| `--mode <full\|headless>` / `--headless` | `RIG_MODE` | Install mode (`full`; `headless` coming soon) |
| `-y`, `--yes`, `--non-interactive` | `RIG_YES=1` | Skip interactive prompts and use flags/defaults |
| `--version <ver>` | `RIG_VERSION` | Pin GitHub Release version (default: latest) |
| `--repo <owner/repo>` | `RIG_REPO` | Override GitHub Releases repository |
| `--uninstall` | — | Stop and remove Rig (keeps `/var/lib/rig` and `/etc/rig`) |

Running `install.sh` again on an existing server upgrades in place after backing up `/var/lib/rig/rig.db` to `/var/lib/rig/rig.db.pre-upgrade-<time>`.

## 2. Create the owner account

Open the printed address (`https://rig.example.com` or `https://<vm-ip>`), paste the one-time setup token (valid 30 minutes), and choose an email and password. Lost the token or it expired?

```sh
sudo -u rig rig-server --data-dir /var/lib/rig setup-token
```

## 3. Change dashboard domain or HTTPS later

If you skipped HTTPS during installation (or want to change domains later):

1. Add a DNS **A** record such as `rig.example.com` pointing at the VM.
2. In the dashboard, open **Settings → Network → Dashboard address**, enter
   the name, keep HTTPS on **Automatic** and save.

Caddy fetches a Let's Encrypt certificate and the browser moves to
`https://rig.example.com`. Sessions are then marked secure. Locked yourself
out (bad DNS, typo)?

```sh
sudo -u rig rig-server --data-dir /var/lib/rig dashboard-domain --clear
```

It takes effect within a minute, or at once after `sudo systemctl restart rig`.

No public domain? Pick **Tailscale** (on a VM in your tailnet) or
**Private CA** instead.

## 4. Deploy an app

1. **Projects → New project**, then **New app**.
2. Choose a source:
   - **Image**, e.g. `nginx:alpine` on port 80; or
   - **Git**, e.g. a public repo with a Dockerfile. Without a Dockerfile Rig
     builds it with Railpack, which it downloads (pinned and checksum-checked)
     on first use.
3. Press **Deploy** and watch the build log.
4. On the app's **Domains** tab, add `app.example.com` (another A record to
   the VM). It gets its own certificate.

No domain for the app? Add one with a **Port** (for example 8081) and open
`http://<vm-ip>:8081`, after opening that port in the firewall.

## 5. Customize branding and workspace preferences

- **Organization branding (`Settings → General`, Admin/Owner):** Rename your organization, upload a custom logo or link to a hosted image, pick a workspace accent color, and optionally display an environment badge (such as `Production` or `Homelab`) in the sidebar. The organization name, logo, and accent color also brand the sign-in screen, browser tab title, and favicon.
- **Profile & avatar (`Account` in the user menu):** Update your display name and set your account avatar using built-in presets, an uploaded image, or an image link.
- **Appearance & operator ergonomics (`Account`):** Toggle light/dark/system theme, personal accent color, compact vs. default UI density, rounded/sharp/pill corner style, classic/floating/inset sidebar with hover-peek, default landing route, timestamp style/timezone/clock format, log & terminal font size and line wrapping, and pin frequently used projects directly to the sidebar.

## Day to day

| What | How |
| --- | --- |
| Logs | `journalctl -u rig -f` and `docker logs rig-caddy` (or `sudo -u rig XDG_RUNTIME_DIR=/run/user/$(id -u rig) podman logs rig-caddy` on Podman) |
| Restart | `sudo systemctl restart rig` |
| Settings | `/etc/rig/rig.env`, then restart |
| Back up | `/var/lib/rig`; keep `master.key` safe, it decrypts your secrets |
| CLI | `rig login https://rig.example.com`, then paste a token from **Account → Access tokens** |
| Password reset | `sudo -u rig rig-server --data-dir /var/lib/rig reset-password you@example.com` (generates a 7-day temporary password requiring change on next sign-in, revokes all sessions and API tokens, and clears login rate limits) |
| Two-factor reset | `sudo -u rig rig-server --data-dir /var/lib/rig reset2fa you@example.com` (disables TOTP, removes recovery codes, and signs out all active sessions) |
| Clear dashboard domain | `sudo -u rig rig-server --data-dir /var/lib/rig dashboard-domain --clear` |
| Reset dashboard access | `sudo -u rig rig-server --data-dir /var/lib/rig dashboard-access public` |
| Uninstall | `sudo ./install.sh --uninstall` (keeps `/var/lib/rig`) |

## Security notes

- Rig's own port (7460) only listens on localhost. Caddy is the only public
  entry point.
- On Podman, `rig` runs its own rootless Podman instance in a private
  subordinate UID/GID range, with no root-equivalent group membership.
- On Docker, the `rig` user belongs to the `docker` group, which is equivalent
  to root on that machine. Treat dashboard owner and admin accounts
  accordingly. Apps never get the engine socket.
- Managed app and database containers run with `no-new-privileges`, `NET_RAW`
  dropped, and a default 512 PID ceiling (`pids_limit`). Local container bridge
  subnets (`docker*`, `br-*`, `podman*`, `cni*`) are automatically excluded from
  LAN and Tailscale access checks.
- Database backups written to `/var/lib/rig/backups/` are encrypted at rest with
  chunked XChaCha20-Poly1305 using `master.key` and decrypted on the fly during
  download or restore.
- The installer mounts `/run/tailscale` into `rig-caddy` only when Tailscale
  TLS or Tailscale access mode is selected.
- The systemd unit runs Rig without capabilities, with a read-only system
  and `/var/lib/rig` as the only writable path.
