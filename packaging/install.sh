#!/usr/bin/env bash
# Installs (or upgrades) Rig on this server from an unpacked release bundle.
#
#   sudo ./install.sh              install or upgrade, then print the setup link
#   sudo ./install.sh --uninstall  stop and remove Rig (keeps /var/lib/rig)
#
# What it sets up:
#   - a container engine: Docker if it is already installed, otherwise rootless
#     Podman (RIG_ENGINE=docker|podman overrides; an upgrade keeps the engine
#     the first install picked)
#   - git, and the engine, from the distribution's own packages, if missing
#   - a `rig` system user, /var/lib/rig, /etc/rig/rig.env
#       Docker: rig joins the docker group
#       Podman: rig runs its own rootless Podman (lingering user, user socket,
#               subordinate IDs); unprivileged ports start at 80 for Caddy
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
DROPIN_DIR=/etc/systemd/system/rig.service.d
SYSCTL_FILE=/etc/sysctl.d/60-rig-ports.conf
CADDY_NAME=rig-caddy
# Pinned so an upgrade never pulls a different proxy by surprise.
# Spelled out in full: Podman refuses short image names.
CADDY_IMAGE=${RIG_CADDY_IMAGE:-docker.io/library/caddy:2.11.9-alpine}
LISTEN=127.0.0.1:7460

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[33mwarning:\033[0m %s\n' "$*" >&2; }
die() { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }

[ "$(id -u)" -eq 0 ] || die "run this as root (sudo $0)"
command -v systemctl >/dev/null || die "systemd is required"

# The engine an earlier install recorded, if any.
recorded_engine() {
  [ -f "$ENV_FILE" ] || return 0
  sed -n 's/^RIG_ENGINE=//p' "$ENV_FILE" | tail -1
}

# Runs the engine's CLI the way Rig uses it: Docker as root, Podman as the
# rig user against its own rootless storage.
engine() {
  if [ "$ENGINE" = podman ]; then
    (cd / && runuser -u rig -- env XDG_RUNTIME_DIR="/run/user/$(id -u rig)" podman "$@")
  else
    docker "$@"
  fi
}
user_systemctl() {
  (cd / && runuser -u rig -- env XDG_RUNTIME_DIR="/run/user/$(id -u rig)" systemctl --user "$@")
}

uninstall() {
  say "stopping Rig"
  systemctl disable --now rig.service 2>/dev/null || true
  rm -f "$UNIT"
  rm -rf "$DROPIN_DIR"
  systemctl daemon-reload
  ENGINE=$(recorded_engine)
  if [ -z "$ENGINE" ]; then
    ENGINE=docker
    command -v docker >/dev/null || ENGINE=podman
  fi
  if [ "$ENGINE" = podman ] && ! id rig >/dev/null 2>&1; then
    :
  elif command -v "$ENGINE" >/dev/null; then
    engine rm -f "$CADDY_NAME" >/dev/null 2>&1 || true
  fi
  rm -f "$BIN_DIR/rig-server" "$BIN_DIR/rig"
  say "removed. Your data is still in $DATA_DIR and $CONF_DIR; apps' containers were left running."
  if [ "$ENGINE" = podman ]; then
    say "delete them with: sudo -u rig XDG_RUNTIME_DIR=/run/user/\$(id -u rig) podman ps -a --filter label=rig.managed"
    say "then: loginctl disable-linger rig && rm -rf $DATA_DIR $CONF_DIR $SYSCTL_FILE"
  else
    say "delete them with: docker ps -a --filter label=rig.managed && rm -rf $DATA_DIR $CONF_DIR"
  fi
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
# Version of a package the distribution would install, or nothing.
candidate_version() {
  case "${ID:-} ${ID_LIKE:-}" in
    *debian* | *ubuntu*)
      apt-get update -qq >/dev/null 2>&1 || true
      apt-cache policy "$1" 2>/dev/null | awk '/Candidate:/ { print $2 }' | grep -v '(none)' ;;
    *fedora* | *rhel* | *centos*)
      dnf -q repoquery --latest-limit 1 --qf '%{version}' "$1" 2>/dev/null | tail -1 ;;
  esac
}
# True when version $1 (Debian/RPM style, e.g. 4.9.3+ds1-1) is 4.9 or later.
podman_new_enough() {
  local v=${1#*:}
  v=${v%%[-+~]*}
  [ "$(printf '%s\n4.9\n' "$v" | sort -V | head -1)" = 4.9 ]
}

# --- engine --------------------------------------------------------------------
ENGINE=${RIG_ENGINE:-$(recorded_engine)}
if [ -n "$ENGINE" ]; then
  previous=$(recorded_engine)
  if [ -n "$previous" ] && [ "$ENGINE" != "$previous" ]; then
    die "Rig was installed on $previous; switching engines in place is not supported (RIG_ENGINE=$ENGINE)"
  fi
elif command -v docker >/dev/null; then
  ENGINE=docker
elif command -v podman >/dev/null; then
  ENGINE=podman
elif podman_new_enough "$(candidate_version podman)"; then
  ENGINE=podman
else
  say "$PRETTY_NAME's Podman is older than 4.9; using Docker instead"
  ENGINE=docker
fi
case "$ENGINE" in docker | podman) ;; *) die "RIG_ENGINE must be docker or podman, not $ENGINE" ;; esac
say "container engine: $ENGINE"

if [ "$ENGINE" = docker ]; then
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
else
  # Railpack builds run in a throwaway BuildKit container; no extra tools.
  if ! command -v podman >/dev/null; then
    say "installing Podman from $PRETTY_NAME's packages"
    case "${ID:-} ${ID_LIKE:-}" in
      *debian* | *ubuntu*)
        install_pkgs podman uidmap dbus-user-session
        install_pkgs passt 2>/dev/null || install_pkgs slirp4netns 2>/dev/null || true ;;
      *fedora* | *rhel* | *centos*) install_pkgs podman ;;
      *) die "install Podman 4.9 or later first, then re-run" ;;
    esac
  fi
  have=$(podman version --format '{{.Client.Version}}' 2>/dev/null || true)
  podman_new_enough "$have" || die "Rig needs Podman 4.9 or later; this server has ${have:-an unknown version}"
  command -v loginctl >/dev/null || die "rootless Podman needs systemd-logind (loginctl)"
fi

# --- user, files ---------------------------------------------------------------
if ! id rig >/dev/null 2>&1; then
  say "creating the rig system user"
  useradd --system --home-dir "$DATA_DIR" --shell /usr/sbin/nologin rig
fi
if [ "$ENGINE" = docker ]; then
  # The docker group is root-equivalent; Rig needs it to run containers. Apps
  # never get the engine socket.
  usermod -aG docker rig
fi

install -d -m 0700 -o rig -g rig "$DATA_DIR"
install -d -m 0750 -o root -g rig "$CONF_DIR"

if [ "$ENGINE" = podman ]; then
  # Rootless Podman maps each container's users onto a private ID range.
  # System users get none by default.
  new_ids=0
  for f in /etc/subuid /etc/subgid; do
    touch "$f"
    if ! grep -q '^rig:' "$f"; then
      new_ids=1
      start=$(awk -F: '{ e = $2 + $3; if (e > m) m = e } END { print (m > 100000 ? m : 100000) }' "$f")
      echo "rig:$start:65536" >> "$f"
    fi
  done
  # Podman 4.x's rootless-netns bind-mounts an empty directory over the first
  # existing parent of /var/lib/cni. Before the first reboot creates
  # /var/lib/cni/networks via tmpfiles.d, that parent is /var/lib, which hides
  # /var/lib/rig and breaks custom networks.
  install -d -m 0755 /var/lib/cni/networks
  # Caddy runs as the rig user and has to bind 80 and 443.
  echo 'net.ipv4.ip_unprivileged_port_start=80' > "$SYSCTL_FILE"
  sysctl -q -w net.ipv4.ip_unprivileged_port_start=80
  # Lingering starts rig's user manager at boot, which starts the Podman
  # socket and brings back containers (podman-restart.service).
  loginctl enable-linger rig
  rig_uid=$(id -u rig)
  for _ in $(seq 1 30); do
    [ -d "/run/user/$rig_uid" ] && user_systemctl is-system-running >/dev/null 2>&1 && break
    user_systemctl is-system-running 2>/dev/null | grep -Eq 'running|degraded' && break
    sleep 1
  done
  [ -d "/run/user/$rig_uid" ] || die "the rig user's session did not start (loginctl user-status rig)"
  # Picks up new ID ranges. It stops every container, so only after adding them.
  [ "$new_ids" = 1 ] && { engine system migrate >/dev/null 2>&1 || true; }
  user_systemctl enable --now podman.socket >/dev/null 2>&1 \
    || die "could not start the Podman socket for rig (journalctl --user-unit podman.socket)"
  user_systemctl enable podman-restart.service >/dev/null 2>&1 \
    || warn "podman-restart.service is missing; containers will not come back after a reboot"
  engine info >/dev/null 2>&1 || die "rootless Podman does not work for the rig user (sudo -u rig podman info)"
  SOCKET=/run/user/$rig_uid/podman/podman.sock
else
  SOCKET=/var/run/docker.sock
fi

port_owner() { ss -Hltnp "sport = :$1" 2>/dev/null | grep -v "$CADDY_NAME" | head -1; }
if ! engine inspect "$CADDY_NAME" >/dev/null 2>&1; then
  for p in 80 443; do
    if [ -n "$(port_owner "$p")" ]; then
      die "port $p is already in use ($(port_owner "$p")); stop that service first"
    fi
  done
fi

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
# Name the engine outright, so a second engine installed later never takes
# over. Older installs predate these lines; fill them in.
grep -q '^RIG_ENGINE=' "$ENV_FILE" || echo "RIG_ENGINE=$ENGINE" >> "$ENV_FILE"
grep -q '^RIG_ENGINE_SOCKET=' "$ENV_FILE" || echo "RIG_ENGINE_SOCKET=unix://$SOCKET" >> "$ENV_FILE"

# --- proxy -------------------------------------------------------------------
# Podman has no daemon to honour unless-stopped across reboots;
# podman-restart.service starts `always` containers.
restart=unless-stopped
[ "$ENGINE" = podman ] && restart=always
if [ "$(engine inspect -f '{{.Config.Image}}' "$CADDY_NAME" 2>/dev/null)" != "$CADDY_IMAGE" ]; then
  say "starting the proxy ($CADDY_IMAGE)"
  engine rm -f "$CADDY_NAME" >/dev/null 2>&1 || true
  ts_mount=()
  [ -d /run/tailscale ] && ts_mount=(-v /run/tailscale:/run/tailscale)
  # Host networking so Caddy sees each visitor's real address. Its config is
  # pushed by Rig; --resume restores the last one across restarts.
  engine run -d --name "$CADDY_NAME" --restart "$restart" --network host \
    --label rig.system=proxy \
    -v rig-caddy-data:/data -v rig-caddy-config:/config "${ts_mount[@]}" \
    "$CADDY_IMAGE" caddy run --resume >/dev/null
elif [ "$(engine inspect -f '{{.State.Running}}' "$CADDY_NAME" 2>/dev/null)" != true ]; then
  engine start "$CADDY_NAME" >/dev/null
fi
for _ in $(seq 1 30); do
  curl -fsS -o /dev/null http://127.0.0.1:2019/config/ 2>/dev/null && break
  sleep 1
done
curl -fsS -o /dev/null http://127.0.0.1:2019/config/ || die "the proxy did not start ($( [ "$ENGINE" = podman ] && echo "sudo -u rig podman" || echo docker) logs $CADDY_NAME)"

# --- service -----------------------------------------------------------------
say "installing the rig service"
install -m 0644 "$HERE/rig.service" "$UNIT"
install -d -m 0755 "$DROPIN_DIR"
if [ "$ENGINE" = podman ]; then
  cat > "$DROPIN_DIR/engine.conf" <<EOF
# Written by install.sh: Rig talks to the rig user's rootless Podman.
[Unit]
After=user@$rig_uid.service
Wants=user@$rig_uid.service

[Service]
# ProtectHome hides /run/user; let the Podman socket through.
ProtectHome=tmpfs
BindReadOnlyPaths=/run/user/$rig_uid/podman
EOF
else
  cat > "$DROPIN_DIR/engine.conf" <<EOF
# Written by install.sh: Rig talks to Docker through the docker group.
[Unit]
After=docker.service
Requires=docker.service

[Service]
SupplementaryGroups=docker
EOF
fi
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
