#!/usr/bin/env bash
# Installs (or upgrades) Rig on this server from an unpacked release bundle.
#
#   sudo ./install.sh              install or upgrade, then print the setup link
#   sudo ./install.sh --uninstall  stop and remove Rig (keeps /var/lib/rig)
#
# What it sets up:
#   - Docker and git from the distribution's own packages, if missing
#   - a `rig` system user (member of the docker group), /var/lib/rig, /etc/rig/rig.env
#   - Caddy as the `rig-caddy` container on the host network (ports 80 and 443)
#   - the `rig` systemd service, listening on 127.0.0.1:7460 behind Caddy
#
# Supported: Debian 12+, Ubuntu 24.04+, Fedora 40+ / RHEL 9+ (x86_64).
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
BIN_DIR=/usr/local/bin
DATA_DIR=/var/lib/rig
CONF_DIR=/etc/rig
ENV_FILE=$CONF_DIR/rig.env
UNIT=/etc/systemd/system/rig.service
CADDY_NAME=rig-caddy
# Pinned so an upgrade never pulls a different proxy by surprise.
CADDY_IMAGE=${RIG_CADDY_IMAGE:-caddy:2.11.9-alpine}
LISTEN=127.0.0.1:7460

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[33mwarning:\033[0m %s\n' "$*" >&2; }
die() { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }

[ "$(id -u)" -eq 0 ] || die "run this as root (sudo $0)"
command -v systemctl >/dev/null || die "systemd is required"

uninstall() {
  say "stopping Rig"
  systemctl disable --now rig.service 2>/dev/null || true
  rm -f "$UNIT"
  systemctl daemon-reload
  docker rm -f "$CADDY_NAME" >/dev/null 2>&1 || true
  rm -f "$BIN_DIR/rig-server" "$BIN_DIR/rig"
  say "removed. Your data is still in $DATA_DIR and $CONF_DIR; apps' containers were left running."
  say "delete them with: docker ps -a --filter label=rig.managed && rm -rf $DATA_DIR $CONF_DIR"
}
if [ "${1:-}" = "--uninstall" ]; then uninstall; exit 0; fi

for f in rig-server rig rig.service; do
  [ -f "$HERE/$f" ] || die "missing $f next to install.sh; run it from the unpacked bundle"
done

# --- preflight ---------------------------------------------------------------
[ "$(uname -m)" = x86_64 ] || die "this bundle is for x86_64, not $(uname -m)"
. /etc/os-release
mem_mb=$(awk '/MemTotal/ { print int($2 / 1024) }' /proc/meminfo)
[ "$mem_mb" -ge 900 ] || warn "only ${mem_mb} MB of memory; Rig and a couple of apps want 1 GB or more"
mkdir -p "$DATA_DIR"
disk_gb=$(df -Pk "$DATA_DIR" | awk 'NR == 2 { print int($4 / 1048576) }')
[ "$disk_gb" -ge 5 ] || warn "only ${disk_gb} GB free for $DATA_DIR; images and builds need room"

port_owner() { ss -Hltnp "sport = :$1" 2>/dev/null | grep -v "$CADDY_NAME" | head -1; }
if ! docker inspect "$CADDY_NAME" >/dev/null 2>&1; then
  for p in 80 443; do
    if [ -n "$(port_owner "$p")" ]; then
      die "port $p is already in use ($(port_owner "$p")); stop that service first"
    fi
  done
fi

# --- packages ----------------------------------------------------------------
install_pkgs() {
  case "${ID:-} ${ID_LIKE:-}" in
    *debian* | *ubuntu*)
      DEBIAN_FRONTEND=noninteractive apt-get update -qq
      DEBIAN_FRONTEND=noninteractive apt-get install -y -qq "$@" >/dev/null ;;
    *fedora* | *rhel* | *centos*)
      dnf install -y -q "$@" ;;
    *) die "unsupported distribution ($PRETTY_NAME); install $* yourself and re-run" ;;
  esac
}
for tool in curl git; do
  command -v "$tool" >/dev/null || { say "installing $tool"; install_pkgs "$tool" ca-certificates; }
done
if ! command -v docker >/dev/null; then
  say "installing Docker from $PRETTY_NAME's packages"
  case "${ID:-} ${ID_LIKE:-}" in
    *debian* | *ubuntu*) install_pkgs docker.io ;;
    *fedora*) install_pkgs moby-engine ;;
    *) die "install Docker Engine first (https://docs.docker.com/engine/install/), then re-run" ;;
  esac
fi
# buildx is only needed for Railpack builds (repositories without a Dockerfile).
if ! docker buildx version >/dev/null 2>&1; then
  case "${ID:-} ${ID_LIKE:-}" in
    *debian* | *ubuntu*) install_pkgs docker-buildx 2>/dev/null || true ;;
    *fedora*) install_pkgs docker-buildx 2>/dev/null || true ;;
  esac
  docker buildx version >/dev/null 2>&1 \
    || warn "docker buildx is not available; repositories without a Dockerfile will not build"
fi
systemctl enable --now docker >/dev/null 2>&1 || true
docker info >/dev/null 2>&1 || die "Docker is installed but not running (systemctl status docker)"

# --- user, files ---------------------------------------------------------------
if ! id rig >/dev/null 2>&1; then
  say "creating the rig system user"
  useradd --system --home-dir "$DATA_DIR" --shell /usr/sbin/nologin rig
fi
# The docker group is root-equivalent; Rig needs it to run containers. Apps
# never get the engine socket.
usermod -aG docker rig

install -d -m 0700 -o rig -g rig "$DATA_DIR"
install -d -m 0750 -o root -g rig "$CONF_DIR"

upgrading=0
systemctl is-active --quiet rig.service && upgrading=1
if [ "$upgrading" = 1 ]; then
  say "backing up the database before upgrading"
  stamp=$(date +%Y%m%d-%H%M%S)
  systemctl stop rig.service
  [ -f "$DATA_DIR/rig.db" ] && install -m 0600 -o rig -g rig "$DATA_DIR/rig.db" "$DATA_DIR/rig.db.pre-upgrade-$stamp"
fi

say "installing binaries to $BIN_DIR"
install -m 0755 "$HERE/rig-server" "$BIN_DIR/rig-server"
install -m 0755 "$HERE/rig" "$BIN_DIR/rig"

public_ip() {
  ip -4 route get 1.1.1.1 2>/dev/null | awk '{ for (i = 1; i < NF; i++) if ($i == "src") { print $(i + 1); exit } }'
}
if [ ! -f "$ENV_FILE" ]; then
  addr=${RIG_PUBLIC_ADDRESS:-$(public_ip)}
  cat > "$ENV_FILE" <<EOF
# Rig settings. Restart after editing: systemctl restart rig
RIG_DATA_DIR=$DATA_DIR
RIG_LISTEN=$LISTEN
RIG_CADDY_ADMIN=http://127.0.0.1:2019
RIG_PROXY_DASHBOARD=true
RIG_PUBLIC_URL=http://${addr:-<this-server>}
# Set once the dashboard is served over HTTPS only:
# RIG_SECURE_COOKIES=true
EOF
  chown root:rig "$ENV_FILE"
  chmod 0640 "$ENV_FILE"
fi

# --- proxy -------------------------------------------------------------------
if [ "$(docker inspect -f '{{.Config.Image}}' "$CADDY_NAME" 2>/dev/null)" != "$CADDY_IMAGE" ]; then
  say "starting the proxy ($CADDY_IMAGE)"
  docker rm -f "$CADDY_NAME" >/dev/null 2>&1 || true
  ts_mount=()
  [ -d /run/tailscale ] && ts_mount=(-v /run/tailscale:/run/tailscale)
  # Host networking so Caddy sees each visitor's real address. Its config is
  # pushed by Rig; --resume restores the last one across restarts.
  docker run -d --name "$CADDY_NAME" --restart unless-stopped --network host \
    --label rig.system=proxy \
    -v rig-caddy-data:/data -v rig-caddy-config:/config "${ts_mount[@]}" \
    "$CADDY_IMAGE" caddy run --resume >/dev/null
fi
for _ in $(seq 1 30); do
  curl -fsS -o /dev/null http://127.0.0.1:2019/config/ 2>/dev/null && break
  sleep 1
done
curl -fsS -o /dev/null http://127.0.0.1:2019/config/ || die "the proxy did not start (docker logs $CADDY_NAME)"

# --- service -----------------------------------------------------------------
say "installing the rig service"
install -m 0644 "$HERE/rig.service" "$UNIT"
systemctl daemon-reload
systemctl enable --now rig.service >/dev/null 2>&1
systemctl restart rig.service

for _ in $(seq 1 30); do
  curl -fsS -o /dev/null "http://$LISTEN/api/v1/setup" 2>/dev/null && break
  sleep 1
done
curl -fsS -o /dev/null "http://$LISTEN/api/v1/setup" \
  || die "Rig did not start; see: journalctl -u rig -n 50"

# shellcheck disable=SC1090
url=$(. "$ENV_FILE"; echo "$RIG_PUBLIC_URL")
if token=$(runuser -u rig -- "$BIN_DIR/rig-server" --data-dir "$DATA_DIR" setup-token 2>/dev/null); then
  cat <<EOF

  Rig is running.

  Open $url and enter this one-time setup token (valid 30 minutes):

      $token

  Need a new token later?  sudo -u rig rig-server --data-dir $DATA_DIR setup-token
EOF
else
  say "Rig is $( [ "$upgrading" = 1 ] && echo upgraded || echo running ) at $url"
fi
cat <<EOF

  Logs:      journalctl -u rig -f
  Settings:  $ENV_FILE
  Data:      $DATA_DIR (back it up; master.key decrypts your secrets)

EOF
