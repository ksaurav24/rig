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

## 1. Build the bundle (on your machine)

```sh
scripts/package.sh
# -> dist/rig-0.1.0-x86_64-linux.tar.gz
```

The binaries are built inside Debian 12, so they run on every distribution
listed above.

## 2. Install (on the VM)

```sh
scp dist/rig-0.1.0-x86_64-linux.tar.gz you@your-vm:
ssh you@your-vm
tar xzf rig-0.1.0-x86_64-linux.tar.gz && cd rig-0.1.0-x86_64-linux
sudo ./install.sh
```

The installer:

- installs Docker and git from the distribution's own packages, if missing;
- creates a `rig` system user, `/var/lib/rig` (data) and `/etc/rig/rig.env`
  (settings);
- starts Caddy as the `rig-caddy` container on ports 80 and 443;
- installs and starts the `rig` systemd service;
- prints the address to open and a one-time setup token (valid 30 minutes).

Running it again upgrades in place, after copying the database to
`/var/lib/rig/rig.db.pre-upgrade-<time>`.

## 3. Create the owner account

Open the printed address (`http://<vm-ip>`), paste the setup token and choose
an email and password. Lost the token or it expired?

```sh
sudo -u rig rig-server --data-dir /var/lib/rig setup-token
```

## 4. Put the dashboard on HTTPS (recommended)

Until it has a domain, the dashboard is plain HTTP on the VM's address, so
your password crosses the network unencrypted. To fix that:

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

## 5. Deploy an app

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

## Day to day

| What | How |
| --- | --- |
| Logs | `journalctl -u rig -f` and `docker logs rig-caddy` |
| Restart | `sudo systemctl restart rig` |
| Settings | `/etc/rig/rig.env`, then restart |
| Back up | `/var/lib/rig`; keep `master.key` safe, it decrypts your secrets |
| CLI | `rig login https://rig.example.com`, then paste a token from **Settings → API tokens** |
| Two-factor reset | `sudo -u rig rig-server --data-dir /var/lib/rig reset2fa you@example.com` |
| Uninstall | `sudo ./install.sh --uninstall` (keeps `/var/lib/rig`) |

## Security notes

- Rig's own port (7460) only listens on localhost. Caddy is the only public
  entry point.
- The `rig` user belongs to the `docker` group, which is equivalent to root
  on that machine. Treat dashboard owner and admin accounts accordingly.
  Apps never get the Docker socket.
- The systemd unit runs Rig without capabilities, with a read-only system
  and `/var/lib/rig` as the only writable path.
