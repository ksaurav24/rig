#!/usr/bin/env bash
# Installs a release bundle on a throwaway "VM" and deploys an app through it.
#
# The VM is a privileged container running systemd, with no container engine
# of its own, so install.sh runs exactly as on a fresh server: apt installs the
# engine and git, the rig user and systemd unit are created, Caddy binds 80/443.
#
#   scripts/package.sh && scripts/vm-test.sh [dist/rig-*.tar.gz]
#   VM_ENGINE=podman scripts/vm-test.sh   rootless Podman on Ubuntu 24.04
#                                         (VM_BASE=debian:trixie for Podman 5)
#   KEEP=1 scripts/vm-test.sh     leave the VM running (docker exec -it rig-vm bash)
#   ENGINE=podman scripts/vm-test.sh      host the VM on Podman instead of Docker
#
# Needs docker (or podman), curl and jq, and network access for apt and image pulls.
set -uo pipefail
cd "$(dirname "$0")/.."

BUNDLE=${1:-$(ls -t dist/rig-*-x86_64-linux.tar.gz 2>/dev/null | head -1)}
[ -f "$BUNDLE" ] || { echo "no bundle; run scripts/package.sh first" >&2; exit 2; }
ENGINE=${ENGINE:-docker}
[ "$ENGINE" = podman ] && docker() { command podman "$@"; }
VM_ENGINE=${VM_ENGINE:-docker}
case "$VM_ENGINE" in
  docker) VM_BASE=${VM_BASE:-docker.io/library/debian:bookworm} ;;
  podman) VM_BASE=${VM_BASE:-docker.io/library/ubuntu:24.04} ;;
  *) echo "VM_ENGINE must be docker or podman" >&2; exit 2 ;;
esac
VM=rig-vm
IMAGE=rig-vm-base-$VM_ENGINE
HTTP_PORT=${VM_HTTP_PORT:-18480}
HTTPS_PORT=${VM_HTTPS_PORT:-18443}
fails=0
pass() { printf '  ok    %s\n' "$1"; }
fail() { printf '  FAIL  %s\n' "$1"; fails=$((fails + 1)); return 1; }
check() { if [ "$2" = "$3" ]; then pass "$1"; else fail "$1 (got '$2', wanted '$3')"; fi; }
vm() { docker exec "$VM" "$@"; }
# The engine's CLI inside the VM, as Rig uses it.
vm_engine() {
  if [ "$VM_ENGINE" = podman ]; then
    vm sh -c 'cd / && runuser -u rig -- env XDG_RUNTIME_DIR=/run/user/$(id -u rig) podman "$@"' podman "$@"
  else
    vm docker "$@"
  fi
}
wait_for() { # seconds, label, command...
  local n=$1 label=$2; shift 2
  for _ in $(seq 1 "$n"); do "$@" >/dev/null 2>&1 && { pass "$label"; return 0; }; sleep 1; done
  fail "timed out: $label"
}

cleanup() {
  docker rm -f "$VM" >/dev/null 2>&1
  docker volume rm -f rig-vm-docker rig-vm-rig >/dev/null 2>&1
}
cleanup
[ -n "${KEEP:-}" ] || trap cleanup EXIT

echo "== VM image ($VM_BASE + systemd, for $VM_ENGINE)"
docker build -q -t "$IMAGE" - >/dev/null <<EOF || { echo "image build failed" >&2; exit 2; }
FROM $VM_BASE
RUN apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
      systemd systemd-sysv dbus iproute2 curl ca-certificates jq procps >/dev/null \
 && rm -rf /var/lib/apt/lists/* \
 && systemctl mask getty.target console-getty.service systemd-firstboot.service
STOPSIGNAL SIGRTMIN+3
CMD ["/sbin/init"]
EOF
pass "image built"

docker run -d --name "$VM" --hostname rig-vm --privileged --cgroupns=private \
  --tmpfs /run --tmpfs /run/lock \
  -v rig-vm-docker:/var/lib/docker -v rig-vm-rig:/var/lib/rig \
  -p "127.0.0.1:$HTTP_PORT:80" \
  -p "127.0.0.1:$HTTPS_PORT:443" \
  "$IMAGE" >/dev/null
wait_for 30 "systemd booted" sh -c "docker exec $VM systemctl is-system-running 2>/dev/null | grep -Eq 'running|degraded'"

echo "== install"
docker cp "$BUNDLE" "$VM:/root/rig.tar.gz"
vm sh -c 'cd /root && tar xzf rig.tar.gz'
DIR=$(vm sh -c 'ls -d /root/rig-*-x86_64-linux')
check "no other engine in the VM" "$(vm sh -c 'command -v docker podman' | wc -l)" 0
check "--help works" "$(vm sh -c "cd $DIR && ./install.sh --help >/dev/null && echo ok")" ok
check "--headless rejected cleanly" "$(vm sh -c "cd $DIR && ./install.sh --headless 2>&1 | grep -qi 'coming soon' && echo ok")" ok
check "conflicting engine flags rejected" "$(vm sh -c "cd $DIR && ./install.sh --podman --docker 2>&1 | grep -qi 'conflicting' && echo ok")" ok
check "--tls auto rejects bare IP" "$(vm sh -c "cd $DIR && ./install.sh --yes --domain 203.0.113.10 --tls auto 2>&1 | grep -qi 'domain name' && echo ok")" ok
if vm sh -c "cd $DIR && RIG_PUBLIC_ADDRESS=localhost:$HTTP_PORT ./install.sh" > /tmp/rig-vm-install.log 2>&1; then
  pass "install.sh succeeded"
else
  fail "install.sh failed"; tail -30 /tmp/rig-vm-install.log; exit 1
fi
TOKEN=$(grep -A2 'one-time setup token' /tmp/rig-vm-install.log | awk 'NF == 1 { print $1; exit }')
check "setup token printed" "$([ -n "$TOKEN" ] && echo yes)" yes
check "rig service active" "$(vm systemctl is-active rig)" active
check "runs as the rig user" "$(vm ps -o user= -C rig-server | head -1 | tr -d ' ')" rig
check "data dir is private" "$(vm stat -c %a /var/lib/rig)" 700
check "master key is private" "$(vm stat -c %a /var/lib/rig/master.key)" 600
check "Rig listens on localhost only" "$(vm ss -Hltn 'sport = :7460' | awk '{ print $4 }')" 127.0.0.1:7460
check "installer picked $VM_ENGINE" "$(vm sed -n 's/^RIG_ENGINE=//p' /etc/rig/rig.env)" "$VM_ENGINE"
other=docker; [ "$VM_ENGINE" = docker ] && other=podman
check "the other engine was not installed" "$(vm sh -c "command -v $other" | wc -l)" 0
check "proxy container running" "$(vm_engine inspect -f '{{.State.Running}}' rig-caddy)" true

echo "== setup through the proxy"
BASE=http://localhost:$HTTP_PORT
JAR=$(mktemp)
wait_for 30 "dashboard served on port 80" sh -c "curl -fsS $BASE/ | grep -q '<div id=\"root\"'"
check "setup is pending" "$(curl -s "$BASE/api/v1/setup" | jq -r .setup_complete)" false
code=$(curl -s -o /tmp/rig-vm-setup.json -w '%{http_code}' -c "$JAR" -H 'content-type: application/json' \
  -H "origin: $BASE" -X POST "$BASE/api/v1/setup" \
  -d "{\"token\":\"$TOKEN\",\"email\":\"owner@example.com\",\"name\":\"Owner\",\"password\":\"correct horse battery staple\",\"org_name\":\"VM\"}")
check "owner created" "$code" 200 || cat /tmp/rig-vm-setup.json
CSRF=$(jq -r .csrf_token /tmp/rig-vm-setup.json)
call() { # method path [body]
  curl -s -b "$JAR" -H 'content-type: application/json' -H "origin: $BASE" -H "x-rig-csrf: $CSRF" \
    -X "$1" "$BASE$2" ${3:+-d "$3"}
}
check "signed in" "$(call GET /api/v1/auth/me | jq -r .user.email)" owner@example.com
check "sessions not marked secure over plain HTTP" "$(grep -c '#HttpOnly_.*TRUE.*rig_session' "$JAR")" 0
check "Rig reports $VM_ENGINE as its engine" "$(call GET /api/v1/system | jq -r .engine)" "$VM_ENGINE"

echo "== deploy an app"
PROJECT=$(call POST /api/v1/projects '{"name":"Demo"}' | jq -r .id)
APP=$(call POST /api/v1/apps "{\"project_id\":\"$PROJECT\",\"name\":\"Hello\",\"source_kind\":\"image\",\"image\":\"nginx:alpine\",\"port\":80}" | jq -r .id)
check "app created" "$([ "${#APP}" -gt 10 ] && echo yes)" yes
call POST "/api/v1/apps/$APP/domains" '{"host":"hello.vm.test","tls_mode":"off"}' >/dev/null
DEP=$(call POST "/api/v1/apps/$APP/deploy" | jq -r .id)
wait_for 180 "deployment running" sh -c "curl -s -b $JAR $BASE/api/v1/deployments/$DEP | jq -e '.status == \"running\"'"
wait_for 30 "app served by host name" sh -c "curl -fsS -H 'host: hello.vm.test' $BASE/ | grep -q 'Welcome to nginx'"
check "dashboard still served for other names" "$(curl -s -o /dev/null -w '%{http_code}' "$BASE/api/v1/setup")" 200

echo "== dashboard on its own domain"
call PUT /api/v1/system/dashboard-domain '{"host":"rig.vm.test","tls_mode":"off"}' | jq -e '.dashboard_domain.host == "rig.vm.test"' >/dev/null \
  && pass "dashboard domain saved" || fail "dashboard domain saved"
wait_for 20 "dashboard served on its domain" sh -c "curl -fsS -H 'host: rig.vm.test' $BASE/api/v1/setup | jq -e .setup_complete"
check "app still served beside it" "$(curl -s -o /dev/null -w '%{http_code}' -H 'host: hello.vm.test' "$BASE/")" 200
vm runuser -u rig -- rig-server --data-dir /var/lib/rig dashboard-domain --clear >/dev/null
vm systemctl restart rig
wait_for 30 "console escape hatch restores the bare address" sh -c "curl -fsS $BASE/api/v1/setup | jq -e .setup_complete"

echo "== survives a reboot of the services"
if [ "$VM_ENGINE" = docker ]; then
  vm systemctl restart docker
  wait_for 60 "rig back after the engine restarts" sh -c "docker exec $VM systemctl is-active rig | grep -qx active && curl -fsS $BASE/api/v1/setup"
  wait_for 60 "app served again" sh -c "curl -fsS -H 'host: hello.vm.test' $BASE/ | grep -q 'Welcome to nginx'"
fi

echo "== survives a reboot of the VM"
docker restart -t 30 "$VM" >/dev/null
wait_for 60 "systemd booted again" sh -c "docker exec $VM systemctl is-system-running 2>/dev/null | grep -Eq 'running|degraded'"
wait_for 90 "rig back after the reboot" sh -c "docker exec $VM systemctl is-active rig | grep -qx active && curl -fsS $BASE/api/v1/setup"
wait_for 90 "app served after the reboot" sh -c "curl -fsS -H 'host: hello.vm.test' $BASE/ | grep -q 'Welcome to nginx'"
check "still signed in after the reboot" "$(curl -s -b "$JAR" "$BASE/api/v1/auth/me" | jq -r .user.email)" owner@example.com

echo "== CLI password reset"
NEW_PW=$(vm runuser -u rig -- rig-server --data-dir /var/lib/rig reset-password owner@example.com | awk '/New password:/ { getline; getline; gsub(/^ +| +$/, ""); print; exit }')
check "new random password generated" "$([ "${#NEW_PW}" -ge 10 ] && echo yes)" yes
check "old session invalidated after password reset" "$(curl -s -o /dev/null -w '%{http_code}' -b "$JAR" "$BASE/api/v1/auth/me")" 401
check "old password rejected" "$(curl -s -o /dev/null -w '%{http_code}' -H 'content-type: application/json' -H "origin: $BASE" -X POST "$BASE/api/v1/auth/login" -d '{"email":"owner@example.com","password":"correct horse battery staple"}')" 401
code=$(curl -s -o /tmp/rig-vm-login.json -w '%{http_code}' -c "$JAR" -H 'content-type: application/json' \
  -H "origin: $BASE" -X POST "$BASE/api/v1/auth/login" \
  -d "{\"email\":\"owner@example.com\",\"password\":\"$NEW_PW\"}")
check "signed in with new random password" "$code" 200 || cat /tmp/rig-vm-login.json
CSRF=$(jq -r .csrf_token /tmp/rig-vm-login.json)

echo "== upgrade in place"
check "upgrade rejects --domain flag" "$(vm sh -c "cd $DIR && ./install.sh --domain other.vm.test 2>&1 | grep -qi 'dashboard-domain' && echo ok")" ok
if vm sh -c "cd $DIR && ./install.sh" > /tmp/rig-vm-upgrade.log 2>&1; then pass "re-running install.sh upgrades"; else fail "upgrade failed"; tail -20 /tmp/rig-vm-upgrade.log; fi
check "database backed up" "$(vm sh -c 'ls /var/lib/rig/rig.db.pre-upgrade-* 2>/dev/null | wc -l')" 1
wait_for 30 "still signed in after upgrade" sh -c "curl -s -b $JAR $BASE/api/v1/auth/me | jq -e '.user.email == \"owner@example.com\"'"
wait_for 30 "app still served after upgrade" sh -c "curl -fsS -H 'host: hello.vm.test' $BASE/ | grep -q 'Welcome to nginx'"

echo "== uninstall"
vm sh -c "cd $DIR && ./install.sh --uninstall" >/dev/null 2>&1
check "service removed" "$(vm systemctl is-active rig 2>/dev/null)" inactive
check "data kept" "$(vm test -f /var/lib/rig/rig.db && echo yes)" yes

echo "== fresh HTTPS install via CLI flags"
vm rm -rf /etc/rig /var/lib/rig/rig.db /var/lib/rig/rig.db-wal /var/lib/rig/rig.db-shm /var/lib/rig/master.key
if vm sh -c "cd $DIR && ./install.sh --$VM_ENGINE --domain rig.vm.test --tls internal --access public --yes" > /tmp/rig-vm-https-install.log 2>&1; then
  pass "HTTPS flag install succeeded"
else
  fail "HTTPS flag install failed"; tail -30 /tmp/rig-vm-https-install.log; exit 1
fi
HTTPS_TOKEN=$(grep -A2 'one-time setup token' /tmp/rig-vm-https-install.log | awk 'NF == 1 { print $1; exit }')
check "HTTPS setup token printed" "$([ -n "$HTTPS_TOKEN" ] && echo yes)" yes
check "RIG_PUBLIC_URL is https" "$(vm sed -n 's/^RIG_PUBLIC_URL=//p' /etc/rig/rig.env)" "https://rig.vm.test"
check "RIG_SECURE_COOKIES enabled" "$(vm sed -n 's/^RIG_SECURE_COOKIES=//p' /etc/rig/rig.env)" "true"
HTTPS_BASE="https://rig.vm.test:$HTTPS_PORT"
HTTPS_JAR=$(mktemp)
wait_for 30 "dashboard served over HTTPS (internal CA)" sh -c "curl -kfsS --resolve rig.vm.test:$HTTPS_PORT:127.0.0.1 $HTTPS_BASE/api/v1/setup | jq -e '.setup_complete == false'"
code=$(curl -ks -o /tmp/rig-vm-https-setup.json -w '%{http_code}' -c "$HTTPS_JAR" \
  --resolve "rig.vm.test:$HTTPS_PORT:127.0.0.1" \
  -H 'content-type: application/json' -H "origin: $HTTPS_BASE" \
  -X POST "$HTTPS_BASE/api/v1/setup" \
  -d "{\"token\":\"$HTTPS_TOKEN\",\"email\":\"https@example.com\",\"name\":\"HTTPS Owner\",\"password\":\"correct horse battery staple\",\"org_name\":\"TLS VM\"}")
check "owner created over HTTPS" "$code" 200 || cat /tmp/rig-vm-https-setup.json
check "session cookie marked Secure over HTTPS" "$(grep -c '#HttpOnly_.*TRUE.*rig_session' "$HTTPS_JAR")" 1
rm -f "$HTTPS_JAR"

rm -f "$JAR"
if [ "$fails" -eq 0 ]; then echo "all VM checks passed"; else echo "$fails VM check(s) failed"; exit 1; fi
