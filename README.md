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
(CLI flags, HTTPS, password reset, first deploy, backups, upgrades) is in
[docs/deploy-on-a-server.md](docs/deploy-on-a-server.md).

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
the server embeds `web/dist`.
