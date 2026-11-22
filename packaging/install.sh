#!/usr/bin/env bash
# Installs (or upgrades) Rig on this server — either from an unpacked release
# bundle or directly via `curl -fsSL <release-url>/install.sh | sudo bash`.
#
#   sudo ./install.sh                      interactive wizard (or upgrade in place)
#   sudo ./install.sh --podman --domain rig.example.com
#   sudo ./install.sh --docker --yes       non-interactive install with defaults
#   sudo ./install.sh --uninstall          stop and remove Rig (keeps /var/lib/rig)
#   ./install.sh --help                    show all flags and environment variables
#
# Supported: Debian 12+, Ubuntu 24.04+, Fedora 40+ / RHEL 9+ (x86_64).
set -euo pipefail

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
DEFAULT_REPO=ksaurav24/rig

say()  { printf '\033[1m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[33mwarning:\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }

usage() {
  cat <<'EOF'
Rig installer — installs or upgrades Rig on a Linux server.

Usage:
  sudo ./install.sh [options]
  curl -fsSL https://github.com/ksaurav24/rig/releases/latest/download/install.sh | sudo bash
  curl -fsSL https://github.com/ksaurav24/rig/releases/latest/download/install.sh | sudo bash -s -- [options]

Options:
  --podman                         Use rootless Podman as the container engine
  --docker                         Use Docker as the container engine
  --engine <podman|docker>         Explicit container engine (env: RIG_ENGINE)
  --mode <full|headless>           Install mode: full (default), headless (coming soon)
  --headless                       Shorthand for --mode headless
  --domain <host>                  Dashboard domain or IP (env: RIG_DOMAIN)
  --tls <auto|internal|tailscale|off>
                                   Dashboard HTTPS mode (env: RIG_TLS)
                                     auto      Let's Encrypt (requires domain DNS)
                                     tailscale Tailscale cert (requires Tailscale)
                                     internal  Self-signed Private CA (IP or LAN host)
                                     off       Plain HTTP (not recommended)
  --access <public|lan|tailscale>  Who may reach the dashboard (env: RIG_ACCESS)
  -y, --yes, --non-interactive     Skip interactive wizard; use flags & defaults (env: RIG_YES=1)
  --version <ver>                  Release version to download, e.g. 0.1.0 (env: RIG_VERSION)
  --repo <owner/repo>              GitHub repo for releases (default: ksaurav24/rig, env: RIG_REPO)
  --uninstall                      Stop and remove Rig (keeps /var/lib/rig and /etc/rig)
  -h, --help                       Show this help and exit

Recovery commands (after install):
  sudo -u rig rig-server --data-dir /var/lib/rig setup-token
  sudo -u rig rig-server --data-dir /var/lib/rig reset-password <email>
  sudo -u rig rig-server --data-dir /var/lib/rig reset2fa <email>
  sudo -u rig rig-server --data-dir /var/lib/rig dashboard-domain --clear
  sudo -u rig rig-server --data-dir /var/lib/rig dashboard-access public
EOF
}

# --- parse CLI arguments -----------------------------------------------------
OPT_ENGINE=""
OPT_MODE=""
OPT_DOMAIN=""
OPT_TLS=""
OPT_ACCESS=""
OPT_VERSION=""
OPT_REPO=""
NON_INTERACTIVE=0
DO_UNINSTALL=0

set_engine_opt() {
  local want=$1
  case "$want" in docker | podman) ;; *) die "invalid engine '$want' (expected podman or docker)" ;; esac
  if [ -n "$OPT_ENGINE" ] && [ "$OPT_ENGINE" != "$want" ]; then
    die "conflicting engine options: both '$OPT_ENGINE' and '$want' were specified"
  fi
  OPT_ENGINE=$want
}

need_val() {
  [ $# -ge 2 ] && [ -n "$2" ] || die "option $1 requires a value"
}

while [ $# -gt 0 ]; do
  case "$1" in
    -h | --help) usage; exit 0 ;;
    --uninstall) DO_UNINSTALL=1; shift ;;
    -y | --yes | --non-interactive) NON_INTERACTIVE=1; shift ;;
    --podman) set_engine_opt podman; shift ;;
    --docker) set_engine_opt docker; shift ;;
    --engine) need_val "$@"; set_engine_opt "$2"; shift 2 ;;
    --engine=*) set_engine_opt "${1#*=}"; shift ;;
    --headless) OPT_MODE=headless; shift ;;
    --mode) need_val "$@"; OPT_MODE=$2; shift 2 ;;
    --mode=*) OPT_MODE=${1#*=}; shift ;;
    --domain) need_val "$@"; OPT_DOMAIN=$2; shift 2 ;;
    --domain=*) OPT_DOMAIN=${1#*=}; shift ;;
    --tls) need_val "$@"; OPT_TLS=$2; shift 2 ;;
    --tls=*) OPT_TLS=${1#*=}; shift ;;
    --access) need_val "$@"; OPT_ACCESS=$2; shift 2 ;;
    --access=*) OPT_ACCESS=${1#*=}; shift ;;
    --version) need_val "$@"; OPT_VERSION=$2; shift 2 ;;
    --version=*) OPT_VERSION=${1#*=}; shift ;;
    --repo) need_val "$@"; OPT_REPO=$2; shift 2 ;;
    --repo=*) OPT_REPO=${1#*=}; shift ;;
    *) die "unknown option '$1' (run with --help for usage)" ;;
  esac
done

# Environment fallbacks (explicit CLI flags win).
OPT_ENGINE=${OPT_ENGINE:-${RIG_ENGINE:-}}
OPT_MODE=${OPT_MODE:-${RIG_MODE:-full}}
OPT_DOMAIN=${OPT_DOMAIN:-${RIG_DOMAIN:-}}
OPT_TLS=${OPT_TLS:-${RIG_TLS:-}}
OPT_ACCESS=${OPT_ACCESS:-${RIG_ACCESS:-}}
OPT_VERSION=${OPT_VERSION:-${RIG_VERSION:-}}
OPT_REPO=${OPT_REPO:-${RIG_REPO:-$DEFAULT_REPO}}
[ "${RIG_YES:-0}" = 1 ] && NON_INTERACTIVE=1

case "$OPT_MODE" in
  full) ;;
  headless) die "headless mode is coming soon and is not available yet" ;;
  *) die "invalid mode '$OPT_MODE' (expected full or headless)" ;;
esac
if [ -n "$OPT_ENGINE" ]; then
  case "$OPT_ENGINE" in docker | podman) ;; *) die "RIG_ENGINE / --engine must be docker or podman, not '$OPT_ENGINE'" ;; esac
fi
if [ -n "$OPT_TLS" ]; then
  case "$OPT_TLS" in auto | internal | tailscale | off) ;; *) die "invalid --tls '$OPT_TLS' (expected auto, internal, tailscale or off)" ;; esac
fi
if [ -n "$OPT_ACCESS" ]; then
  case "$OPT_ACCESS" in public | lan | tailscale) ;; *) die "invalid --access '$OPT_ACCESS' (expected public, lan or tailscale)" ;; esac
fi

# Temp resource cleanup.
TMP_CLEANUP=()
cleanup_tmp() {
  local p
  for p in "${TMP_CLEANUP[@]:-}"; do
    if [ -n "$p" ]; then
      rm -rf "$p"
    fi
  done
}
trap cleanup_tmp EXIT

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
if [ "$DO_UNINSTALL" = 1 ]; then uninstall; exit 0; fi

# --- preflight & package helpers ---------------------------------------------
[ "$(uname -m)" = x86_64 ] || die "Rig release bundles are currently for x86_64, not $(uname -m)"
# shellcheck disable=SC1091
. /etc/os-release

install_pkgs() {
  case "${ID:-} ${ID_LIKE:-}" in
    *debian* | *ubuntu*)
      DEBIAN_FRONTEND=noninteractive apt-get update -qq
      DEBIAN_FRONTEND=noninteractive apt-get install -y -qq "$@" >/dev/null ;;
    *fedora* | *rhel* | *centos*)
      dnf install -y -q "$@" ;;
    *) die "unsupported distribution (${PRETTY_NAME:-unknown}); install $* yourself and re-run" ;;
  esac
}

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
  [ -n "${1:-}" ] || return 1
  local v=${1#*:}
  v=${v%%[-+~]*}
  [ "$(printf '%s\n4.9\n' "$v" | sort -V | head -1)" = 4.9 ]
}

docker_is_installable() {
  case "${ID:-} ${ID_LIKE:-}" in
    *debian* | *ubuntu* | *fedora*) echo true ;;
    *) echo false ;;
  esac
}

public_ip() {
  ip -4 route get 1.1.1.1 2>/dev/null | awk '{ for (i = 1; i < NF; i++) if ($i == "src") { print $(i + 1); exit } }'
}

# Extract Tailscale MagicDNS hostname without requiring jq.
tailscale_dns_name() {
  command -v tailscale >/dev/null 2>&1 || return 0
  local raw
  raw=$(tailscale status --json --peers=false 2>/dev/null) || return 0
  printf '%s' "$raw" | grep -q '"BackendState"[[:space:]]*:[[:space:]]*"Running"' || return 0
  printf '%s' "$raw" \
    | sed -n 's/.*"DNSName"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
    | head -1 \
    | sed 's/\.$//'
}

# --- locate or download release bundle ---------------------------------------
HERE=""
if [ -n "${BASH_SOURCE[0]:-}" ] && [ -f "${BASH_SOURCE[0]}" ]; then
  candidate_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
  if [ -f "$candidate_dir/rig-server" ] && [ -f "$candidate_dir/rig" ] && [ -f "$candidate_dir/rig.service" ]; then
    HERE=$candidate_dir
  fi
fi

if [ -z "$HERE" ]; then
  for tool in curl tar; do
    command -v "$tool" >/dev/null || { say "installing $tool"; install_pkgs "$tool" ca-certificates; }
  done
  if [ -n "$OPT_VERSION" ]; then
    ver=${OPT_VERSION#v}
    tag="v$ver"
  else
    say "finding latest Rig release from github.com/$OPT_REPO"
    effective=$(curl -fsSL -o /dev/null -w '%{url_effective}' "https://github.com/$OPT_REPO/releases/latest") \
      || die "could not look up latest release from https://github.com/$OPT_REPO/releases/latest"
    tag=${effective##*/}
    [ -n "$tag" ] && [ "$tag" != "latest" ] && [ "$tag" != "releases" ] \
      || die "no published releases found in $OPT_REPO (pass --version <ver> or run from an unpacked release bundle)"
    ver=${tag#v}
  fi
  tarball="rig-${ver}-x86_64-linux.tar.gz"
  base_url="https://github.com/$OPT_REPO/releases/download/$tag"
  dl_dir=$(mktemp -d)
  TMP_CLEANUP+=("$dl_dir")
  say "downloading $tarball ($tag)"
  curl -fsSL -o "$dl_dir/$tarball" "$base_url/$tarball" \
    || die "failed to download $base_url/$tarball"
  curl -fsSL -o "$dl_dir/$tarball.sha256" "$base_url/$tarball.sha256" \
    || die "failed to download $base_url/$tarball.sha256"
  (cd "$dl_dir" && sha256sum -c "$tarball.sha256" >/dev/null) \
    || die "SHA-256 verification failed for $tarball"
  tar -xzf "$dl_dir/$tarball" -C "$dl_dir"
  HERE="$dl_dir/rig-${ver}-x86_64-linux"
fi

for f in rig-server rig rig.service; do
  [ -f "$HERE/$f" ] || die "missing $f in bundle ($HERE)"
done

mem_mb=$(awk '/MemTotal/ { print int($2 / 1024) }' /proc/meminfo)
mkdir -p "$DATA_DIR"
disk_gb=$(df -Pk "$DATA_DIR" | awk 'NR == 2 { print int($4 / 1048576) }')

# --- upgrade vs. fresh install planning --------------------------------------
existing_install=0
[ -f "$ENV_FILE" ] && existing_install=1

PLAN_ENGINE=""
PLAN_MODE="full"
PLAN_DOMAIN=""
PLAN_TLS="off"
PLAN_ACCESS="public"
PLAN_PUBLIC_URL=""
PLAN_SECURE_COOKIES="false"

if [ "$existing_install" = 1 ]; then
  if [ -n "$OPT_DOMAIN" ] || [ -n "$OPT_TLS" ] || [ -n "$OPT_ACCESS" ]; then
    die "Rig is already installed; change dashboard domain or access with 'sudo -u rig rig-server --data-dir $DATA_DIR dashboard-domain ...' or 'dashboard-access ...'"
  fi
  previous=$(recorded_engine)
  if [ -n "$OPT_ENGINE" ] && [ -n "$previous" ] && [ "$OPT_ENGINE" != "$previous" ]; then
    die "Rig was installed on $previous; switching engines in place is not supported (--engine=$OPT_ENGINE)"
  fi
  PLAN_ENGINE=${OPT_ENGINE:-$previous}
  if [ -z "$PLAN_ENGINE" ]; then
    if command -v docker >/dev/null; then
      PLAN_ENGINE=docker
    elif command -v podman >/dev/null; then
      PLAN_ENGINE=podman
    else
      PLAN_ENGINE=docker
    fi
  fi
else
  # Probe installed and candidate engines before deciding whether to run the TUI.
  docker_ver=""
  if command -v docker >/dev/null 2>&1; then
    docker_ver=$(docker version --format '{{.Server.Version}}' 2>/dev/null || docker --version 2>/dev/null | awk '{print $3}' | tr -d ',' || echo installed)
  fi
  docker_can=$(docker_is_installable)
  podman_ver=""
  podman_cand=""
  if command -v podman >/dev/null 2>&1; then
    podman_ver=$(podman version --format '{{.Client.Version}}' 2>/dev/null || echo unknown)
    if ! podman_new_enough "$podman_ver"; then
      podman_cand=$(candidate_version podman || true)
    fi
  else
    podman_cand=$(candidate_version podman || true)
  fi

  has_tty=0
  if ( : </dev/tty >/dev/tty ) 2>/dev/null; then
    has_tty=1
  fi
  all_specified=0
  if [ -n "$OPT_ENGINE" ] && { [ -n "$OPT_DOMAIN" ] || [ -n "$OPT_TLS" ]; } && [ -n "$OPT_ACCESS" ]; then
    all_specified=1
  fi

  if [ "$NON_INTERACTIVE" != 1 ] && [ "$has_tty" = 1 ] && [ "$all_specified" != 1 ]; then
    plan_file=$(mktemp)
    TMP_CLEANUP+=("$plan_file")
    wizard_args=(
      --output "$plan_file"
      --distro-name "${PRETTY_NAME:-Linux}"
      --ram-mb "$mem_mb"
      --disk-gb "$disk_gb"
      --docker-installed "$docker_ver"
      --docker-installable "$docker_can"
      --podman-installed "$podman_ver"
      --podman-candidate "$podman_cand"
    )
    [ -n "$OPT_ENGINE" ] && wizard_args+=(--engine "$OPT_ENGINE")
    [ -n "$OPT_DOMAIN" ] && wizard_args+=(--domain "$OPT_DOMAIN")
    [ -n "$OPT_TLS" ] && wizard_args+=(--tls "$OPT_TLS")
    [ -n "$OPT_ACCESS" ] && wizard_args+=(--access "$OPT_ACCESS")

    set +e
    "$HERE/rig-server" install-wizard "${wizard_args[@]}" </dev/tty >/dev/tty 2>/dev/tty
    rc=$?
    set -e
    if [ "$rc" -eq 130 ]; then
      exit 130
    elif [ "$rc" -ne 0 ]; then
      exit "$rc"
    fi
    # shellcheck disable=SC1090
    . "$plan_file"
  else
    # Non-interactive fresh install (CI, vm-test.sh, --yes, or all flags passed).
    [ "$mem_mb" -ge 900 ] || warn "only ${mem_mb} MB of memory; Rig and a couple of apps want 1 GB or more"
    [ "$disk_gb" -ge 5 ] || warn "only ${disk_gb} GB free for $DATA_DIR; images and builds need room"

    if [ -n "$OPT_ENGINE" ]; then
      PLAN_ENGINE=$OPT_ENGINE
    elif [ -n "$docker_ver" ]; then
      PLAN_ENGINE=docker
    elif [ -n "$podman_ver" ] && podman_new_enough "$podman_ver"; then
      PLAN_ENGINE=podman
    elif podman_new_enough "$podman_cand"; then
      PLAN_ENGINE=podman
    else
      [ -n "$podman_cand" ] && say "${PRETTY_NAME:-This OS}'s Podman is older than 4.9; using Docker instead"
      PLAN_ENGINE=docker
    fi

    if [ -n "$OPT_TLS" ]; then
      PLAN_TLS=$OPT_TLS
    elif [ -n "$OPT_DOMAIN" ]; then
      PLAN_TLS=auto
    else
      PLAN_TLS=off
    fi

    PLAN_DOMAIN=$OPT_DOMAIN
    case "$PLAN_TLS" in
      auto)
        [ -n "$PLAN_DOMAIN" ] || die "--tls auto requires --domain <hostname>"
        if printf '%s' "$PLAN_DOMAIN" | grep -Eq '^[0-9]+(\.[0-9]+){3}$'; then
          die "--tls auto requires a domain name, not an IP address (use --tls internal for an IP)"
        fi
        ;;
      tailscale)
        if [ -z "$PLAN_DOMAIN" ]; then
          PLAN_DOMAIN=$(tailscale_dns_name)
          [ -n "$PLAN_DOMAIN" ] || die "--tls tailscale requires Tailscale to be running with MagicDNS enabled (or pass --domain <name>.ts.net)"
        fi
        ;;
      internal)
        if [ -z "$PLAN_DOMAIN" ]; then
          PLAN_DOMAIN=${RIG_PUBLIC_ADDRESS:-$(public_ip)}
          [ -n "$PLAN_DOMAIN" ] || die "--tls internal requires --domain <host-or-ip> when no IP could be detected"
        fi
        ;;
      off) ;;
    esac

    if [ -n "$OPT_ACCESS" ]; then
      PLAN_ACCESS=$OPT_ACCESS
    elif [ "$PLAN_TLS" = tailscale ]; then
      PLAN_ACCESS=tailscale
    else
      PLAN_ACCESS=public
    fi
    if [ "$PLAN_ACCESS" = tailscale ] && [ -z "$(tailscale_dns_name)" ]; then
      command -v tailscale >/dev/null 2>&1 || die "--access tailscale requires Tailscale to be installed and running"
    fi

    if [ "$PLAN_TLS" != off ]; then
      PLAN_PUBLIC_URL="https://$PLAN_DOMAIN"
      PLAN_SECURE_COOKIES=true
    elif [ -n "$PLAN_DOMAIN" ]; then
      PLAN_PUBLIC_URL="http://$PLAN_DOMAIN"
      PLAN_SECURE_COOKIES=false
    else
      addr=${RIG_PUBLIC_ADDRESS:-$(public_ip)}
      PLAN_PUBLIC_URL="http://${addr:-<this-server>}"
      PLAN_SECURE_COOKIES=false
    fi
  fi
fi

ENGINE=$PLAN_ENGINE
case "$ENGINE" in docker | podman) ;; *) die "engine must be docker or podman, not '$ENGINE'" ;; esac
say "container engine: $ENGINE"

# --- packages & engine -------------------------------------------------------
for tool in curl git; do
  command -v "$tool" >/dev/null || { say "installing $tool"; install_pkgs "$tool" ca-certificates; }
done

if [ "$ENGINE" = docker ]; then
  if ! command -v docker >/dev/null; then
    say "installing Docker from ${PRETTY_NAME:-the distribution}'s packages"
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
  have=""
  if command -v podman >/dev/null; then
    have=$(podman version --format '{{.Client.Version}}' 2>/dev/null || true)
  fi
  if [ -z "$have" ] || ! podman_new_enough "$have"; then
    if [ -n "$have" ]; then
      say "upgrading Podman ($have -> 4.9+) from ${PRETTY_NAME:-the distribution}'s packages"
    else
      say "installing Podman from ${PRETTY_NAME:-the distribution}'s packages"
    fi
    case "${ID:-} ${ID_LIKE:-}" in
      *debian* | *ubuntu*)
        install_pkgs podman uidmap dbus-user-session
        install_pkgs passt 2>/dev/null || install_pkgs slirp4netns 2>/dev/null || true ;;
      *fedora* | *rhel* | *centos*) install_pkgs podman ;;
      *) die "install or upgrade to Podman 4.9 or later first, then re-run" ;;
    esac
  fi
  have=$(podman version --format '{{.Client.Version}}' 2>/dev/null || true)
  podman_new_enough "$have" || die "Rig needs Podman 4.9 or later; this server has ${have:-an unknown version} (upgrade Podman or re-run with --docker)"
  command -v loginctl >/dev/null || die "rootless Podman needs systemd-logind (loginctl)"
fi

# --- user, files -------------------------------------------------------------
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

  if [ "$PLAN_TLS" = tailscale ] && command -v tailscale >/dev/null 2>&1; then
    say "granting the rig user permission to request Tailscale certificates"
    if ! tailscale set --operator=rig >/dev/null 2>&1; then
      warn "could not run 'tailscale set --operator=rig'; if Tailscale HTTPS fails, set TS_PERMIT_CERT_UID=$rig_uid for tailscaled"
    fi
  fi
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

if [ ! -f "$ENV_FILE" ]; then
  secure_line='# RIG_SECURE_COOKIES=true'
  [ "$PLAN_SECURE_COOKIES" = true ] && secure_line='RIG_SECURE_COOKIES=true'
  cat > "$ENV_FILE" <<EOF
# Rig settings. Restart after editing: systemctl restart rig
RIG_DATA_DIR=$DATA_DIR
RIG_LISTEN=$LISTEN
RIG_CADDY_ADMIN=http://127.0.0.1:2019
RIG_PROXY_DASHBOARD=true
RIG_PUBLIC_URL=$PLAN_PUBLIC_URL
$secure_line
EOF
  chown root:rig "$ENV_FILE"
  chmod 0640 "$ENV_FILE"
fi
# Name the engine outright, so a second engine installed later never takes
# over. Older installs predate these lines; fill them in.
grep -q '^RIG_ENGINE=' "$ENV_FILE" || echo "RIG_ENGINE=$ENGINE" >> "$ENV_FILE"
grep -q '^RIG_ENGINE_SOCKET=' "$ENV_FILE" || echo "RIG_ENGINE_SOCKET=unix://$SOCKET" >> "$ENV_FILE"

# Apply fresh-install domain/TLS and access preferences directly to Rig's DB
# before starting the service, so Caddy serves HTTPS from the very first request.
if [ "$existing_install" = 0 ]; then
  if [ -n "$PLAN_DOMAIN" ]; then
    say "configuring dashboard domain ($PLAN_DOMAIN, TLS: $PLAN_TLS)"
    runuser -u rig -- "$BIN_DIR/rig-server" --data-dir "$DATA_DIR" \
      dashboard-domain "$PLAN_DOMAIN" --tls "$PLAN_TLS" >/dev/null
  fi
  if [ "$PLAN_ACCESS" != public ]; then
    say "configuring dashboard access ($PLAN_ACCESS)"
    runuser -u rig -- "$BIN_DIR/rig-server" --data-dir "$DATA_DIR" \
      dashboard-access "$PLAN_ACCESS" >/dev/null
  fi
fi

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

# Wait briefly for Rig's proxy supervisor to push the initial route to Caddy.
for _ in $(seq 1 15); do
  curl -fsS http://127.0.0.1:2019/config/apps/http/servers 2>/dev/null | grep -q 'rig_' && break
  sleep 1
done

# shellcheck disable=SC1090
url=$(. "$ENV_FILE"; echo "$RIG_PUBLIC_URL")
if token=$(runuser -u rig -- "$BIN_DIR/rig-server" --data-dir "$DATA_DIR" setup-token 2>/dev/null); then
  cat <<EOF

  Rig is running.

  Open $url and enter this one-time setup token (valid 30 minutes):

      $token
EOF
  if [ "$existing_install" = 0 ] && [ "$PLAN_TLS" = tailscale ]; then
    cat <<'EOF'

  Note: make sure HTTPS Certificates are enabled in your Tailscale admin console.
EOF
  elif [ "$existing_install" = 0 ] && [ "$PLAN_TLS" = internal ]; then
    cat <<'EOF'

  Note: your browser will show a one-time certificate warning for the self-signed cert;
        traffic and your setup password are still encrypted.
EOF
  elif [ "$existing_install" = 0 ] && [ "$PLAN_TLS" = off ]; then
    addr=${RIG_PUBLIC_ADDRESS:-$(public_ip)}
    addr=${addr%%:*}
    cat <<EOF

  Tip: to avoid sending your setup password over plain HTTP, tunnel over SSH:
       ssh -L 7460:127.0.0.1:7460 <user>@${addr:-<server-ip>}  then open http://localhost:7460
EOF
  fi
else
  say "Rig is $( [ "$upgrading" = 1 ] && echo upgraded || echo running ) at $url"
fi
cat <<EOF

  Logs:            journalctl -u rig -f
  Settings:        $ENV_FILE
  Data:            $DATA_DIR (back it up; master.key decrypts your secrets)

  Recovery commands:
    New setup token:   sudo -u rig rig-server --data-dir $DATA_DIR setup-token
    Reset password:    sudo -u rig rig-server --data-dir $DATA_DIR reset-password <email>
    Clear domain:      sudo -u rig rig-server --data-dir $DATA_DIR dashboard-domain --clear
    Reset access:      sudo -u rig rig-server --data-dir $DATA_DIR dashboard-access public

EOF
