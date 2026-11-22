#!/usr/bin/env bash
# End-to-end check against the local container engine.
#
# Starts a throwaway Rig (own data dir, own Caddy container on high ports),
# then drives the HTTP API the way the dashboard does: setup, deploy an image,
# reach it through the proxy, redeploy without downtime, roll back, manage a
# database, and clean up. Needs docker, curl and jq.
#
#   scripts/e2e.sh            run everything
#   KEEP=1 scripts/e2e.sh     leave the server running afterwards for poking
#   ENGINE=podman scripts/e2e.sh   run against rootless Podman instead of Docker
set -uo pipefail
cd "$(dirname "$0")/.."

ENGINE=${ENGINE:-docker}
for tool in "$ENGINE" curl jq; do
  command -v "$tool" >/dev/null || { echo "missing: $tool"; exit 2; }
done

WORK=$(mktemp -d "${TMPDIR:-/tmp}/rig-e2e.XXXXXX")
ENGINE_FLAGS=()
if [ "$ENGINE" = podman ]; then
  # Every docker call below goes to Podman, and Rig talks to its API socket.
  docker() { command podman "$@"; }
  # The script's own helper containers use short names; Rig must not need this.
  printf 'unqualified-search-registries = ["docker.io"]\n' >"$WORK/registries.conf"
  export CONTAINERS_REGISTRIES_CONF=$WORK/registries.conf
  ENGINE_FLAGS=(--engine-socket "${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/podman/podman.sock")
fi
API=http://127.0.0.1:7461
CADDY=rig-caddy-e2e
ADMIN=127.0.0.1:37019
HTTP=38080
HTTPS=38443
JAR=$WORK/jar
fails=0

pass() { printf '  ok    %s\n' "$1"; }
fail() { printf '  FAIL  %s\n' "$1"; fails=$((fails + 1)); return 1; }
# check <name> <actual> <expected>
check() { if [ "$2" = "$3" ]; then pass "$1"; else fail "$1 (got '$2', wanted '$3')"; fi; }

cleanup() {
  [ -n "${SERVER_PID:-}" ] && kill "$SERVER_PID" 2>/dev/null
  [ -n "${MOCK_PID:-}" ] && kill "$MOCK_PID" 2>/dev/null
  [ -n "${SINK_PID:-}" ] && kill "$SINK_PID" 2>/dev/null
  docker rm -f "$CADDY" >/dev/null 2>&1
  [ -n "${APP:-}" ] && docker ps -aq -f "label=rig.app=$APP" | xargs -r "$ENGINE" rm -f >/dev/null 2>&1
  [ -n "${DB:-}" ] && docker ps -aq -f "label=rig.database=$DB" | xargs -r "$ENGINE" rm -f >/dev/null 2>&1
  rm -rf "$WORK"
}
[ -n "${KEEP:-}" ] || trap cleanup EXIT

# call <method> <path> [json]  -> body on stdout, status in $WORK/status
call() {
  local method=$1 path=$2 body=${3:-}
  local args=(-s -o "$WORK/body" -w '%{http_code}' -X "$method" -b "$JAR" -c "$JAR"
    -H "origin: $API" -H "x-rig-csrf: ${CSRF:-}")
  [ -n "$body" ] && args+=(-H 'content-type: application/json' -d "$body")
  curl "${args[@]}" "$API$path" >"$WORK/status"
  cat "$WORK/body"
}
status() { cat "$WORK/status"; }

# wait_for <seconds> <description> <command...>
wait_for() {
  local limit=$1 what=$2; shift 2
  for _ in $(seq 1 "$limit"); do
    if "$@" >/dev/null 2>&1; then return 0; fi
    sleep 1
  done
  fail "timed out: $what"
  return 1
}
app_status_is() { [ "$(call GET "/api/v1/apps/$1" | jq -r .status)" = "$2" ]; }
deployment_status_is() { [ "$(call GET "/api/v1/deployments/$1" | jq -r .status)" = "$2" ]; }
db_status_is() { [ "$(call GET "/api/v1/databases/$1" | jq -r .status)" = "$2" ]; }
via_proxy() { curl -s -o /dev/null -w '%{http_code}' -H "host: $1" "http://127.0.0.1:$HTTP/"; }

echo "== build and start"
cargo build -q -p rig-server -p rig-cli || exit 2
docker rm -f "$CADDY" >/dev/null 2>&1
printf '{"admin":{"listen":"%s"}}' "$ADMIN" >"$WORK/caddy.json"
TS_MOUNT=(); [ -S /run/tailscale/tailscaled.sock ] && TS_MOUNT=(-v /run/tailscale:/run/tailscale)
docker run -d --name "$CADDY" --network host "${TS_MOUNT[@]}" -v "$WORK/caddy.json:/etc/caddy/caddy.json:ro" \
  caddy:2-alpine caddy run --config /etc/caddy/caddy.json >/dev/null || exit 2
# A stand-in for GitHub: plain HTTP API, HTTPS git host with a private repo.
GH_API=28601; GH_GIT=28602; GH="$WORK/gh"; mkdir -p "$GH/acme"
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj /CN=127.0.0.1 -addext subjectAltName=IP:127.0.0.1 \
  -keyout "$GH/tls.key" -out "$GH/tls.crt" 2>/dev/null
openssl genrsa -traditional -out "$GH/app.pem" 2048 2>/dev/null
git init -q -b main "$GH/work" && printf 'FROM nginx:alpine\n' >"$GH/work/Dockerfile"
gh_commit() { echo "$1" >"$GH/work/version" && git -C "$GH/work" add -A && git -C "$GH/work" -c user.email=e2e@x -c user.name=e2e commit -qm "$1" \
  && git -C "$GH/work" push -q "$GH/acme/site.git" main && git -C "$GH/acme/site.git" update-server-info; }
git init -q --bare -b main "$GH/acme/site.git" && gh_commit one
python3 scripts/github-mock.py $GH_API $GH_GIT "$GH" "$GH/tls.crt" "$GH/tls.key" "$GH/app.pem" 2>"$WORK/gh-mock.log" &
MOCK_PID=$!
# git trusts the mock's certificate on top of the system roots (public clones still need those).
for ca in /etc/ssl/certs/ca-certificates.crt /etc/pki/tls/certs/ca-bundle.crt /etc/ssl/cert.pem; do
  [ -f "$ca" ] && { cat "$ca" >"$GH/ca.pem"; break; }
done
cat "$GH/tls.crt" >>"$GH/ca.pem"
GIT_SSL_CAINFO="$GH/ca.pem" \
./target/debug/rig-server --data-dir "$WORK/data" --listen 127.0.0.1:7461 \
  --caddy-admin "http://$ADMIN" --http-port $HTTP --https-port $HTTPS "${ENGINE_FLAGS[@]}" \
  --github-api "http://127.0.0.1:$GH_API" --github-web "https://127.0.0.1:$GH_GIT" --poll-interval 10 \
  >"$WORK/out.log" 2>"$WORK/err.log" &
SERVER_PID=$!
wait_for 20 "server start" curl -sf "$API/api/v1/setup" || { cat "$WORK/err.log"; exit 1; }

echo "== setup and auth"
TOKEN=$(grep -oE '^ {6}\S+' "$WORK/err.log" | tr -d ' ')
call GET /api/v1/auth/me >/dev/null; check "unauthenticated is rejected" "$(status)" 401
call POST /api/v1/setup '{"token":"wrong","org_name":"Acme","name":"Ada","email":"ada@example.com","password":"correct horse 1"}' >/dev/null
check "wrong setup token is rejected" "$(status)" 403
CSRF=$(call POST /api/v1/setup "{\"token\":\"$TOKEN\",\"org_name\":\"Acme\",\"name\":\"Ada\",\"email\":\"ada@example.com\",\"password\":\"correct horse 1\"}" | jq -r .csrf_token)
check "setup succeeds" "$(status)" 200
call POST /api/v1/setup "{\"token\":\"$TOKEN\",\"org_name\":\"A\",\"name\":\"B\",\"email\":\"b@example.com\",\"password\":\"correct horse 1\"}" >/dev/null
check "setup token is single use" "$(status)" 409
SAVED=$CSRF; CSRF=
call POST /api/v1/projects '{"name":"nope"}' >/dev/null; check "write without CSRF is rejected" "$(status)" 403
CSRF=$SAVED
check "data dir is private" "$(stat -c %a "$WORK/data")" 700
check "master key is private" "$(stat -c %a "$WORK/data/master.key")" 600

echo "== deploy an image"
PROJECT=$(call POST /api/v1/projects '{"name":"Shop"}' | jq -r .id)
APP=$(call POST /api/v1/apps "{\"project_id\":\"$PROJECT\",\"name\":\"Web\",\"source_kind\":\"image\",\"image\":\"nginx:alpine\",\"port\":80,\"health_path\":\"/\"}" | jq -r .id)
check "app created" "$(status)" 201
call PUT "/api/v1/apps/$APP/env" '{"key":"API_KEY","value":"s3cret-value","is_secret":true}' >/dev/null
check "secret is masked in the list" "$(call GET "/api/v1/apps/$APP/env" | jq -r '.[0].value')" null
check "secret can be revealed" "$(call POST "/api/v1/apps/$APP/env/API_KEY/reveal" | jq -r .value)" s3cret-value
if grep -aq 's3cret-value' "$WORK"/data/rig.db*; then fail "secret is stored in clear text"; else pass "secret is encrypted on disk"; fi
call POST "/api/v1/apps/$APP/domains" '{"host":"web.test","tls_mode":"off"}' >/dev/null
check "domain added" "$(status)" 201
D1=$(call POST "/api/v1/apps/$APP/deploy" | jq -r .id)
check "deploy accepted" "$(status)" 202
call POST "/api/v1/apps/$APP/deploy" >/dev/null; check "second deploy while busy is refused" "$(status)" 409
wait_for 120 "first deployment running" deployment_status_is "$D1" running
check "app is running" "$(call GET "/api/v1/apps/$APP" | jq -r .status)" running
wait_for 10 "route reaches the app" test "$(via_proxy web.test)" = 200
check "proxy serves the app" "$(via_proxy web.test)" 200
LOGS=$(curl -s -N -b "$JAR" "$API/api/v1/deployments/$D1/logs" --max-time 5)
case $LOGS in *"event: end"*) pass "deployment log stream ends with a result" ;; *) fail "deployment log stream has no end event" ;; esac
CID=$(docker ps -q -f "label=rig.app=$APP")
check "container gets the secret" "$(docker exec "$CID" printenv API_KEY)" s3cret-value
check "container port is bound to loopback only" "$(docker port "$CID" 80/tcp | cut -d: -f1 | sort -u)" 127.0.0.1

echo "== network access"
# Caddy runs on the host network, so it sees each client's real address.
# Connecting to one of this machine's own addresses uses it as the source.
LAN_IP=$(ip -4 -o addr show scope global | awk '$2 != "tailscale0" && $2 !~ /^(docker|podman|br-|veth|cni)/ {split($4,a,"/"); print a[1]; exit}')
TS_IP=$(tailscale ip -4 2>/dev/null | head -1)
from() { curl -s -o /dev/null -w '%{http_code}' -H "host: $2" "http://$1:$HTTP/"; }
call POST "/api/v1/apps/$APP/domains" '{"host":"lan.test","tls_mode":"off","access":"lan"}' >/dev/null
check "LAN-only domain added" "$(status)" 201
call POST "/api/v1/apps/$APP/domains" '{"host":"tail.test","tls_mode":"off","access":"tailscale"}' >/dev/null
check "tailnet-only domain added" "$(status)" 201
call POST "/api/v1/apps/$APP/domains" '{"host":"web.test","tls_mode":"off","port":'$HTTP'}' >/dev/null
check "the proxy's own port is refused" "$(status)" 422
call POST "/api/v1/apps/$APP/domains" '{"host":"x.test","tls_mode":"auto","port":28555}' >/dev/null
check "public certificates on a custom port are refused" "$(status)" 422
call POST "/api/v1/apps/$APP/domains" '{"host":"x.test","tls_mode":"tailscale"}' >/dev/null
check "tailscale certificates outside ts.net are refused" "$(status)" 422
call POST "/api/v1/apps/$APP/domains" '{"host":"port.test","tls_mode":"off","port":28555}' >/dev/null
check "a domain on its own port is added" "$(status)" 201
call POST "/api/v1/apps/$APP/domains" '{"host":"port2.test","tls_mode":"internal","port":28555}' >/dev/null
check "HTTPS and plain HTTP cannot share a port" "$(status)" 422
wait_for 10 "port route" test "$(curl -s -o /dev/null -w '%{http_code}' -H 'host: port.test' http://127.0.0.1:28555/)" = 200
check "the app answers on its own port" "$(curl -s -o /dev/null -w '%{http_code}' -H 'host: port.test' http://127.0.0.1:28555/)" 200
check "loopback reaches a LAN-only domain" "$(via_proxy lan.test)" 200
check "loopback reaches a tailnet-only domain" "$(via_proxy tail.test)" 200
if [ -n "$LAN_IP" ]; then
  check "the LAN reaches a LAN-only domain" "$(from "$LAN_IP" lan.test)" 200
  check "the LAN is refused by a tailnet-only domain" "$(from "$LAN_IP" tail.test)" 403
  check "the LAN reaches a public domain" "$(from "$LAN_IP" web.test)" 200
else echo "  skip  no LAN address"; fi
if [ -n "$TS_IP" ]; then
  check "the tailnet reaches a tailnet-only domain" "$(from "$TS_IP" tail.test)" 200
  check "the tailnet is refused by a LAN-only domain" "$(from "$TS_IP" lan.test)" 403
  TS_NAME=$(tailscale status --json --peers=false 2>/dev/null | jq -r '.Self.DNSName // empty' | sed 's/\.$//')
  if [ -n "$TS_NAME" ] && [ ${#TS_MOUNT[@]} -gt 0 ] && [ -z "${OFFLINE:-}" ]; then
    call POST "/api/v1/apps/$APP/domains" "{\"host\":\"$TS_NAME\",\"tls_mode\":\"tailscale\",\"access\":\"tailscale\",\"port\":28556}" >/dev/null
    check "a ts.net name on its own port is added" "$(status)" 201
    ts_https() { curl -s -o /dev/null -w '%{http_code}' --resolve "$TS_NAME:28556:$1" "https://$TS_NAME:28556/"; }
    wait_for 60 "a trusted ts.net certificate" test "$(ts_https "$TS_IP")" = 200
    check "the tailnet gets HTTPS with a trusted certificate" "$(ts_https "$TS_IP")" 200
  else echo "  skip  ts.net certificate (no MagicDNS name, no tailscaled socket, or OFFLINE)"; fi
else echo "  skip  no tailnet address"; fi
check "network info lists the LAN" "$(call GET /api/v1/system/network | jq -r --arg ip "$LAN_IP" '.lan_addresses | index($ip) != null')" true
call PUT /api/v1/system/dashboard-access '{"access":"lan"}' >/dev/null
check "dashboard can be limited to the LAN" "$(status)" 200
check "the limit shows in network info" "$(call GET /api/v1/system/network | jq -r .dashboard_access)" lan
check "the dashboard still answers on loopback" "$(curl -s -o /dev/null -w '%{http_code}' -b "$JAR" "$API/api/v1/auth/me")" 200
check "console command resets dashboard access" "$(./target/debug/rig-server --data-dir "$WORK/data" dashboard-access public 2>&1 | grep -c 'now public')" 1
call PUT /api/v1/system/dashboard-access '{"access":"public"}' >/dev/null
check "dashboard is public again" "$(status)" 200

echo "== redeploy without downtime, then roll back"
( bad=0; for _ in $(seq 1 60); do [ "$(via_proxy web.test)" = 200 ] || bad=$((bad + 1)); sleep 0.2; done; echo "$bad" >"$WORK/bad" ) &
PROBE=$!
D2=$(call POST "/api/v1/apps/$APP/deploy" | jq -r .id)
wait_for 120 "second deployment running" deployment_status_is "$D2" running
wait "$PROBE"
check "no failed requests during redeploy" "$(cat "$WORK/bad")" 0
check "old deployment is superseded" "$(call GET "/api/v1/deployments/$D1" | jq -r .status)" superseded
check "one container per app" "$(docker ps -q -f "label=rig.app=$APP" | wc -l)" 1
# Move the tag to a different image, the way a registry push would. A
# rollback must still run the image D1 ran. The tag is restored afterwards.
ORIG_IMAGE=$(docker image inspect -f '{{.Id}}' docker.io/library/nginx:alpine)
docker tag docker.io/library/caddy:2-alpine docker.io/library/nginx:alpine
D3=$(call POST "/api/v1/deployments/$D1/rollback" | jq -r .id)
check "rollback accepted" "$(status)" 202
wait_for 120 "rollback running" deployment_status_is "$D3" running
docker tag "$ORIG_IMAGE" docker.io/library/nginx:alpine
check "rollback runs the original image, not the moved tag" \
  "$(docker inspect -f '{{.Image}}' "$(docker ps -q -f "label=rig.deployment=$D3")")" "$ORIG_IMAGE"
curl -s -m 10 -b "$JAR" "$API/api/v1/deployments/$D3/logs" >"$WORK/d3.log"
check "rollback reused the pinned image without pulling" \
  "$(grep -q 'Reusing image' "$WORK/d3.log" && ! grep -q 'Pulling' "$WORK/d3.log" && echo yes)" yes
check "log stream needs sign-in" "$(curl -s -m 5 -o /dev/null -w '%{http_code}' "$API/api/v1/deployments/$D3/logs")" 401
check "log stream of an unknown deployment" "$(curl -s -m 5 -o /dev/null -w '%{http_code}' -b "$JAR" "$API/api/v1/deployments/01aaaaaaaaaaaaaaaaaaaaaaaa/logs")" 404
call POST "/api/v1/deployments/$D3/rollback" >/dev/null; check "live deployment cannot be rolled back to" "$(status)" 422

echo "== alerts"
SINK=$WORK/alerts
python3 -I scripts/alert-listener.py 37090 "$SINK" &
SINK_PID=$!
sleep 0.5
call POST /api/v1/alerts/channels '{"name":"ops","url":"ftp://example.com/x","events":["deploy_failed"]}' >/dev/null
check "non-http alert URL refused" "$(status)" 422
CREATED=$(call POST /api/v1/alerts/channels '{"name":"ops","url":"http://127.0.0.1:37090/hook?token=s3cret","events":["deploy_failed"]}')
check "alert channel created" "$(status)" 201
CHANNEL=$(jq -r .channel.id <<<"$CREATED")
SIGNING=$(jq -r .signing_secret <<<"$CREATED")
check "channel list hides the URL" "$(call GET /api/v1/alerts/channels | grep -c s3cret)" 0
check "test alert delivered" "$(call POST "/api/v1/alerts/channels/$CHANNEL/test" | jq -r .ok)" true
check "test alert signed with the channel secret" "$(cat "$SINK/0.sig")" "sha256=$(openssl dgst -sha256 -hmac "$SIGNING" -hex <"$SINK/0.body" | sed 's/^.*= //')"
check "delivery recorded" "$(call GET /api/v1/alerts/channels | jq -r '.[0].last_sent_at != null')" true

echo "== a failing deploy keeps the live one"
call PATCH "/api/v1/apps/$APP" '{"image":"rig-e2e/does-not-exist:nope"}' >/dev/null
D4=$(call POST "/api/v1/apps/$APP/deploy" | jq -r .id)
wait_for 120 "bad deployment fails" deployment_status_is "$D4" failed
check "app stays running" "$(call GET "/api/v1/apps/$APP" | jq -r .status)" running
check "proxy still serves the app" "$(via_proxy web.test)" 200
alert_sent() { grep -q '"event":"deploy.failed"' "$SINK"/*.body; }
wait_for 20 "failed deploy alerted" alert_sent && pass "failed deploy alerted"
call POST "/api/v1/alerts/channels" '{"name":"broken","url":"http://127.0.0.1:37090/fail-me","events":["disk_low"]}' >/dev/null
BROKEN=$(jq -r .channel.id "$WORK/body")
check "failing receiver reported" "$(call POST "/api/v1/alerts/channels/$BROKEN/test" | jq -r .ok)" false
check "error kept on the channel" "$(call GET /api/v1/alerts/channels | jq -r --arg id "$BROKEN" '.[] | select(.id==$id) | .last_error | test("500")')" true
call PATCH "/api/v1/alerts/channels/$CHANNEL" '{"enabled":false}' >/dev/null
check "channel can be paused" "$(jq -r .enabled "$WORK/body")" false
call DELETE "/api/v1/alerts/channels/$BROKEN" >/dev/null; check "channel removed" "$(status)" 204
kill "$SINK_PID" 2>/dev/null

echo "== push to deploy"
call PATCH "/api/v1/apps/$APP" '{"image":"nginx:alpine"}' >/dev/null
HOOK_SECRET=$(call GET "/api/v1/apps/$APP/webhook" | jq -r .secret)
check "webhook secret readable by developers" "$(status)" 200
hook() { # hook <body> <extra curl args...>
  local body=$1; shift
  curl -s -o "$WORK/hook" -w '%{http_code}' -X POST -H 'content-type: application/json' "$@" -d "$body" "$API/api/v1/hooks/apps/$APP"
}
sig() { printf '%s' "$1" | openssl dgst -sha256 -hmac "$2" -hex | sed 's/^.*= //'; }
PUSH='{"ref":"refs/heads/main"}'
check "unsigned hook refused" "$(hook "$PUSH")" 401
check "wrongly signed hook refused" "$(hook "$PUSH" -H "x-hub-signature-256: sha256=$(sig "$PUSH" wrong)")" 401
check "tampered body refused" "$(hook '{"ref":"refs/heads/evil"}' -H "x-hub-signature-256: sha256=$(sig "$PUSH" "$HOOK_SECRET")")" 401
check "unknown app looks the same" "$(curl -s -o /dev/null -w '%{http_code}' -X POST "$API/api/v1/hooks/apps/nope")" 401
check "ping acknowledged" "$(hook '{}' -H 'x-github-event: ping' -H "x-hub-signature-256: sha256=$(sig '{}' "$HOOK_SECRET")")" 200
check "signed hook deploys" "$(hook "$PUSH" -H 'x-github-event: push' -H "x-hub-signature-256: sha256=$(sig "$PUSH" "$HOOK_SECRET")")" 202
D5=$(jq -r .deployment_id "$WORK/hook")
check "deployment triggered by webhook" "$(call GET "/api/v1/deployments/$D5" | jq -r .trigger)" webhook
wait_for 120 "webhook deployment runs" deployment_status_is "$D5" running
OLD_SECRET=$HOOK_SECRET
HOOK_SECRET=$(call POST "/api/v1/apps/$APP/webhook/rotate" | jq -r .secret)
check "rotated secret differs" "$([ "$HOOK_SECRET" != "$OLD_SECRET" ] && echo yes)" yes
check "old secret stops working" "$(hook "$PUSH" -H "x-rig-signature: sha256=$(sig "$PUSH" "$OLD_SECRET")")" 401
check "rotation is audited" "$(call GET '/api/v1/audit?limit=20' | jq -r '[.[] | select(.action=="app.webhook.rotate")] | length')" 1

echo "== roles"
DEV_TOKEN=$(call POST /api/v1/tokens '{"name":"ci","role":"viewer","expires_in_days":1}' | jq -r .secret)
check "viewer can read" "$(curl -s -o /dev/null -w '%{http_code}' -H "authorization: Bearer $DEV_TOKEN" "$API/api/v1/apps")" 200
check "viewer cannot deploy" "$(curl -s -o /dev/null -w '%{http_code}' -X POST -H "authorization: Bearer $DEV_TOKEN" "$API/api/v1/apps/$APP/deploy")" 403
check "viewer cannot read the webhook secret" "$(curl -s -o /dev/null -w '%{http_code}' -H "authorization: Bearer $DEV_TOKEN" "$API/api/v1/apps/$APP/webhook")" 403
check "viewer cannot list alert channels" "$(curl -s -o /dev/null -w '%{http_code}' -H "authorization: Bearer $DEV_TOKEN" "$API/api/v1/alerts/channels")" 403
check "viewer cannot reveal secrets" "$(curl -s -o /dev/null -w '%{http_code}' -X POST -H "authorization: Bearer $DEV_TOKEN" "$API/api/v1/apps/$APP/env/API_KEY/reveal")" 403
ws() { curl -s -o /dev/null -w '%{http_code}' -m 5 -H 'connection: Upgrade' -H 'upgrade: websocket' \
  -H 'sec-websocket-version: 13' -H 'sec-websocket-key: dGhlIHNhbXBsZSBub25jZQ==' "$@" "$API/api/v1/apps/$APP/terminal"; }
check "terminal needs sign-in" "$(ws)" 401
check "viewer cannot open a shell" "$(ws -H "authorization: Bearer $DEV_TOKEN")" 403
check "terminal refuses a session without Origin" "$(ws -b "$JAR")" 403
check "terminal refuses a foreign Origin" "$(ws -b "$JAR" -H 'origin: http://evil.example')" 403
check "terminal upgrades for the owner" "$(ws -b "$JAR" -H "origin: $API")" 101

echo "== team"
ME=$(call GET /api/v1/auth/me | jq -r .user.id)
ADDED=$(call POST /api/v1/members '{"name":"Grace","email":"Grace@Example.com","role":"developer"}')
MEMBER=$(echo "$ADDED" | jq -r .member.user_id)
TEMP_PW=$(echo "$ADDED" | jq -r .temporary_password)
check "member added with a temporary password" "$(echo "$ADDED" | jq -r '.member.email + " " + .member.role')" "grace@example.com developer"
call POST /api/v1/members '{"name":"Grace","email":"grace@example.com","role":"viewer"}' >/dev/null
check "duplicate email refused" "$(status)" 409
call PATCH "/api/v1/members/$ME" '{"role":"viewer"}' >/dev/null
check "owner cannot demote themselves" "$(status)" 403
call DELETE "/api/v1/members/$ME" >/dev/null
check "owner cannot remove themselves" "$(status)" 403

# Act as the new member in a second cookie jar.
OWNER_JAR=$JAR; OWNER_CSRF=$CSRF
JAR=$WORK/jar2; CSRF=
CSRF=$(call POST /api/v1/auth/login "$(jq -n --arg p "$TEMP_PW" '{email:"grace@example.com",password:$p}')" | jq -r .csrf_token)
check "member signs in with the temporary password" "$(status)" 200
call GET /api/v1/members >/dev/null
check "developer cannot list members" "$(status)" 403
call POST /api/v1/auth/password "$(jq -n --arg p "$TEMP_PW" '{current_password:"wrong wrong wrong",new_password:"a brand new pass"}')" >/dev/null
check "wrong current password refused" "$(status)" 422
call POST /api/v1/auth/password "$(jq -n --arg p "$TEMP_PW" '{current_password:$p,new_password:"short"}')" >/dev/null
check "weak new password refused" "$(status)" 422
call POST /api/v1/auth/password "$(jq -n --arg p "$TEMP_PW" '{current_password:$p,new_password:"a brand new pass"}')" >/dev/null
check "member changes their password" "$(status)" 204
call GET /api/v1/auth/me >/dev/null
check "current session survives the change" "$(status)" 200
MEMBER_TOKEN=$(call POST /api/v1/tokens '{"name":"laptop","role":"developer"}' | jq -r .secret)
check "password change refused over an API token" "$(curl -s -o /dev/null -w '%{http_code}' -X POST -H "authorization: Bearer $MEMBER_TOKEN" -H 'content-type: application/json' -d '{"current_password":"a brand new pass","new_password":"yet another pass"}' "$API/api/v1/auth/password")" 403
MEMBER_JAR=$JAR; MEMBER_CSRF=$CSRF
JAR=$OWNER_JAR; CSRF=$OWNER_CSRF

check "role change applies" "$(call PATCH "/api/v1/members/$MEMBER" '{"role":"viewer"}' | jq -r .role)" viewer
check "demotion reaches their API token at once" "$(curl -s -o /dev/null -w '%{http_code}' -X POST -H "authorization: Bearer $MEMBER_TOKEN" "$API/api/v1/apps/$APP/deploy")" 403
call POST "/api/v1/members/$MEMBER/reset-password" >/dev/null
check "password reset" "$(status)" 200
JAR=$MEMBER_JAR; CSRF=$MEMBER_CSRF
call GET /api/v1/auth/me >/dev/null
check "reset signs the member out" "$(status)" 401
JAR=$OWNER_JAR; CSRF=$OWNER_CSRF
call DELETE "/api/v1/members/$MEMBER" >/dev/null
check "member removed" "$(status)" 204
check "removed member's token stops working" "$(curl -s -o /dev/null -w '%{http_code}' -H "authorization: Bearer $MEMBER_TOKEN" "$API/api/v1/apps")" 401
check "the email can be added again" "$(call POST /api/v1/members '{"name":"Grace","email":"grace@example.com","role":"admin"}' | jq -r .member.role)" admin
check "audit log recorded team changes" "$(call GET /api/v1/audit | jq '[.[] | select(.action | startswith("member."))] | length >= 4')" true

echo "== two-factor sign-in"
# totp <base32 secret> [offset seconds] -> the six-digit code (RFC 6238)
totp() {
  python3 -I -c 'import base64,hmac,hashlib,struct,sys,time
k=base64.b32decode(sys.argv[1]+"="*(-len(sys.argv[1])%8)); t=int((time.time()+int(sys.argv[2]))//30)
d=hmac.new(k,struct.pack(">Q",t),hashlib.sha1).digest(); o=d[-1]&15
print("%06d"%((struct.unpack(">I",d[o:o+4])[0]&0x7fffffff)%1000000))' "$1" "${2:-0}"
}
TFA=$(call POST /api/v1/members '{"name":"Lin","email":"lin@example.com","role":"developer"}')
TFA_PW=$(echo "$TFA" | jq -r .temporary_password)
OWNER_JAR=$JAR; OWNER_CSRF=$CSRF
JAR=$WORK/jar3; CSRF=
CSRF=$(call POST /api/v1/auth/login "$(jq -n --arg p "$TFA_PW" '{email:"lin@example.com",password:$p}')" | jq -r .csrf_token)
check "2fa: off by default" "$(call GET /api/v1/account/totp | jq -r .enabled)" false
call POST /api/v1/account/totp/setup '{"password":"not it at all"}' >/dev/null
check "2fa: setup needs the password" "$(status)" 422
ENROL=$(call POST /api/v1/account/totp/setup "$(jq -n --arg p "$TFA_PW" '{password:$p}')")
SECRET=$(echo "$ENROL" | jq -r .secret)
check "2fa: setup returns a local QR code" "$(echo "$ENROL" | jq -r '.qr_svg | startswith("<?xml")')" true
check "2fa: not on until confirmed" "$(call GET /api/v1/account/totp | jq -r .enabled)" false
call POST /api/v1/account/totp/enable '{"code":"000000"}' >/dev/null
check "2fa: wrong confirming code refused" "$(status)" 422
CODES=$(call POST /api/v1/account/totp/enable "$(jq -n --arg c "$(totp "$SECRET")" '{code:$c}')")
check "2fa: enabled with ten recovery codes" "$(echo "$CODES" | jq '.codes | length')" 10
check "2fa: status shows it on" "$(call GET /api/v1/account/totp | jq -r '"\(.enabled) \(.recovery_codes_left)"')" "true 10"
check "2fa: session survives enabling" "$(call GET /api/v1/auth/me >/dev/null; status)" 200
LIN_TOKEN=$(call POST /api/v1/tokens '{"name":"ci","role":"developer"}' | jq -r .secret)
check "2fa: cannot be turned off over an API token" "$(curl -s -o /dev/null -w '%{http_code}' -X POST -H "authorization: Bearer $LIN_TOKEN" -H 'content-type: application/json' -d '{"password":"x","code":"000000"}' "$API/api/v1/account/totp/disable")" 403
JAR=$WORK/jar4; CSRF=
call POST /api/v1/auth/login "$(jq -n --arg p "$TFA_PW" '{email:"lin@example.com",password:$p}')" >/dev/null
check "2fa: password alone is not enough" "$(status) $(jq -r .error.code "$WORK/body")" "401 second_factor_required"
check "2fa: no session without the code" "$(call GET /api/v1/auth/me >/dev/null; status)" 401
call POST /api/v1/auth/login "$(jq -n --arg p "$TFA_PW" '{email:"lin@example.com",password:$p,code:"123456"}')" >/dev/null
check "2fa: wrong code refused" "$(status) $(jq -r .error.code "$WORK/body")" "401 unauthorized"
call POST /api/v1/auth/login "$(jq -n --arg p "wrong password" --arg c "$(totp "$SECRET")" '{email:"lin@example.com",password:$p,code:$c}')" >/dev/null
check "2fa: a right code does not excuse a wrong password" "$(status)" 401
# The code used to enable is spent; the next step's code (within skew) is fresh.
NEXT=$(totp "$SECRET" 30)
CSRF=$(call POST /api/v1/auth/login "$(jq -n --arg p "$TFA_PW" --arg c "$NEXT" '{email:"lin@example.com",password:$p,code:$c}')" | jq -r .csrf_token)
check "2fa: password and code sign in" "$(status)" 200
JAR=$WORK/jar5
call POST /api/v1/auth/login "$(jq -n --arg p "$TFA_PW" --arg c "$NEXT" '{email:"lin@example.com",password:$p,code:$c}')" >/dev/null
check "2fa: a code cannot be replayed" "$(status)" 401
REC=$(echo "$CODES" | jq -r '.codes[0]')
CSRF=$(call POST /api/v1/auth/login "$(jq -n --arg p "$TFA_PW" --arg c "${REC^^}" '{email:"lin@example.com",password:$p,code:$c}')" | jq -r .csrf_token)
check "2fa: a recovery code signs in" "$(status)" 200
JAR5_CSRF=$CSRF
JAR=$WORK/jar6; CSRF=
call POST /api/v1/auth/login "$(jq -n --arg p "$TFA_PW" --arg c "$REC" '{email:"lin@example.com",password:$p,code:$c}')" >/dev/null
check "2fa: a recovery code works once" "$(status)" 401
JAR=$WORK/jar5; CSRF=$JAR5_CSRF
check "2fa: recovery codes left" "$(call GET /api/v1/account/totp | jq -r .recovery_codes_left)" 9
call POST /api/v1/account/totp/disable "$(jq -n --arg p "$TFA_PW" '{password:$p,code:"000000"}')" >/dev/null
check "2fa: disabling needs a valid code" "$(status)" 422
call POST /api/v1/account/totp/disable "$(jq -n --arg p "$TFA_PW" --arg c "$(echo "$CODES" | jq -r '.codes[1]')" '{password:$p,code:$c}')" >/dev/null
check "2fa: disabled with password and recovery code" "$(status)" 204
JAR=$WORK/jar4
check "2fa: disabling signs out other sessions" "$(call GET /api/v1/auth/me >/dev/null; status)" 401
JAR=$WORK/jar6; CSRF=
call POST /api/v1/auth/login "$(jq -n --arg p "$TFA_PW" '{email:"lin@example.com",password:$p}')" >/dev/null
check "2fa: password alone works again" "$(status)" 200
# Lost authenticator: the console reset.
JAR=$WORK/jar5; CSRF=$JAR5_CSRF
call POST /api/v1/account/totp/setup "$(jq -n --arg p "$TFA_PW" '{password:$p}')" >/dev/null
call POST /api/v1/account/totp/enable "$(jq -n --arg c "$(totp "$SECRET")" '{code:$c}')" >/dev/null
check "2fa: re-enabling with a new secret ignores the old one" "$(status)" 422
SECRET=$(call POST /api/v1/account/totp/setup "$(jq -n --arg p "$TFA_PW" '{password:$p}')" | jq -r .secret)
call POST /api/v1/account/totp/enable "$(jq -n --arg c "$(totp "$SECRET")" '{code:$c}')" >/dev/null
check "2fa: re-enabled" "$(status)" 200
check "2fa: console reset" "$(./target/debug/rig-server --data-dir "$WORK/data" reset2fa lin@example.com >/dev/null 2>&1; echo $?)" 0
check "2fa: off after the console reset" "$(call GET /api/v1/account/totp | jq -r .enabled)" false
JAR=$OWNER_JAR; CSRF=$OWNER_CSRF
check "2fa: changes are audited" "$(call GET /api/v1/audit | jq '[.[] | select(.action | startswith("auth.totp"))] | length')" 4

echo "== databases"
call POST /api/v1/databases "{\"project_id\":\"$PROJECT\",\"name\":\"Web\",\"engine\":\"redis\"}" >/dev/null
check "database cannot take an app's name" "$(status)" 409
call POST /api/v1/apps "{\"project_id\":\"$PROJECT\",\"name\":\"cache\",\"source_kind\":\"image\",\"image\":\"nginx:alpine\",\"port\":80}" >/dev/null
TMP_APP=$(jq -r .id "$WORK/body"); call DELETE "/api/v1/apps/$TMP_APP" >/dev/null
call POST /api/v1/databases "{\"project_id\":\"$PROJECT\",\"name\":\"x\",\"engine\":\"postgres\",\"version\":\"17; rm\"}" >/dev/null
check "unknown version is rejected" "$(status)" 422
DB=$(call POST /api/v1/databases "{\"project_id\":\"$PROJECT\",\"name\":\"Main DB\",\"engine\":\"postgres\"}" | jq -r .id)
check "database accepted" "$(status)" 202
call POST /api/v1/apps "{\"project_id\":\"$PROJECT\",\"name\":\"main-db\",\"source_kind\":\"image\",\"image\":\"nginx:alpine\",\"port\":80}" >/dev/null
check "app cannot take a database's name" "$(status)" 409
wait_for 180 "database running" db_status_is "$DB" running
URL=$(call POST "/api/v1/databases/$DB/reveal" | jq -r .url)
PASSWORD=$(jq -r .password "$WORK/body")
if grep -aq "$PASSWORD" "$WORK"/data/rig.db*; then fail "database password is stored in clear text"; else pass "database password is encrypted on disk"; fi
NET=$(docker ps -q -f "label=rig.database=$DB" | xargs "$ENGINE" inspect -f '{{range $k, $_ := .NetworkSettings.Networks}}{{$k}}{{end}}')
check "reachable by name inside the project" \
  "$(docker run --rm --network "$NET" postgres:17-alpine psql "$URL" -Atc 'create table t(v text); insert into t values (1); select count(*) from t' 2>&1 | tail -1)" 1
call POST "/api/v1/databases/$DB/stop" >/dev/null; check "database stops" "$(status)" 200
check "its container is gone" "$(docker ps -aq -f "label=rig.database=$DB" | wc -l)" 0
call POST "/api/v1/databases/$DB/start" >/dev/null; check "database starts again" "$(status)" 202
wait_for 180 "database running again" db_status_is "$DB" running
check "data survives a restart" \
  "$(docker run --rm --network "$NET" postgres:17-alpine psql "$URL" -Atc 'select count(*) from t' 2>&1 | tail -1)" 1

echo "== backups"
backup_status_is() { [ "$(call GET "/api/v1/databases/$DB/backups" | jq -r --arg id "$1" '.[] | select(.id == $id) | .status')" = "$2" ]; }
BK=$(call POST "/api/v1/databases/$DB/backups" | jq -r .id)
check "backup started" "$(status)" 202
call POST "/api/v1/databases/$DB/backups" >/dev/null
check "second backup while one runs is refused" "$(status)" 409
wait_for 60 "backup finished" backup_status_is "$BK" ok
check "backup is restorable" "$(call GET "/api/v1/databases/$DB/backups" | jq -r '.[0].restorable')" true
BK_FILE=$(ls "$WORK"/data/backups/"$DB"/*)
check "backup file is private" "$(stat -c %a "$BK_FILE")" 600
if grep -aq "$PASSWORD" "$BK_FILE"; then fail "backup holds the password"; else pass "backup holds no password"; fi
curl -s -b "$JAR" -D "$WORK/headers" -o "$WORK/dump" "$API/api/v1/backups/$BK/download"
check "download matches the file" "$(cmp -s "$WORK/dump" "$BK_FILE" && echo same)" same
check "download is an attachment" "$(grep -ci 'content-disposition: attachment' "$WORK/headers")" 1
check "download needs sign-in" "$(curl -s -o /dev/null -w '%{http_code}' "$API/api/v1/backups/$BK/download")" 401
call GET "/api/v1/backups/00000000-0000-0000-0000-000000000000/download" >/dev/null
check "unknown backup is not found" "$(status)" 404
docker run --rm --network "$NET" postgres:17-alpine psql "$URL" -Atc 'insert into t values (2)' >/dev/null 2>&1
call POST "/api/v1/backups/$BK/restore" >/dev/null
check "restore succeeds" "$(status)" 204
check "restore brings back the backed-up rows" \
  "$(docker run --rm --network "$NET" postgres:17-alpine psql "$URL" -Atc 'select count(*) from t' 2>&1 | tail -1)" 1
call DELETE "/api/v1/backups/$BK" >/dev/null
check "backup deleted" "$(status)" 204
check "its file is gone" "$(ls "$WORK"/data/backups/"$DB" | wc -l)" 0
check "audit records the backup work" \
  "$(call GET /api/v1/audit | jq '[.[] | select(.action | startswith("backup.") or . == "database.backup")] | length >= 4')" true

check "databases get a memory ceiling" \
  "$(docker inspect -f '{{.HostConfig.Memory}}' "$(docker ps -q -f "label=rig.database=$DB")")" 1073741824

if [ -z "${OFFLINE:-}" ]; then
  echo "== mongodb"
  MDB=$(call POST /api/v1/databases "{\"project_id\":\"$PROJECT\",\"name\":\"Docs\",\"engine\":\"mongodb\"}" | jq -r .id)
  check "mongodb created" "$(status)" 202
  wait_for 240 "mongodb running" db_status_is "$MDB" running
  MURL=$(call POST "/api/v1/databases/$MDB/reveal" | jq -r .url)
  case "$MURL" in mongodb://*'?authSource=admin') pass "mongodb url has the auth source" ;; *) fail "mongodb url: $MURL" ;; esac
  MC=$(docker ps -q -f "label=rig.database=$MDB")
  mongo() { docker exec -e "U=$(echo "$MURL" | sed 's/@[^:]*:[0-9]*/@127.0.0.1:27017/')" "$MC" sh -c "mongosh \"\$U\" --quiet --eval '$1'"; }
  mongo 'db.t.insertOne({n: 1})' >/dev/null
  MBK=$(call POST "/api/v1/databases/$MDB/backups" | jq -r .id)
  wait_for 90 "mongodb backup finished" sh -c "[ \"\$(curl -s -b '$JAR' '$API/api/v1/databases/$MDB/backups' | jq -r --arg id '$MBK' '.[] | select(.id==\$id) | .status')\" = ok ]"
  if grep -aq "$(echo "$MURL" | sed 's|.*://[^:]*:\([^@]*\)@.*|\1|')" "$WORK"/data/backups/"$MDB"/*; then fail "mongodb backup holds the password"; else pass "mongodb backup keeps the password out"; fi
  mongo 'db.t.insertOne({n: 2})' >/dev/null
  call POST "/api/v1/backups/$MBK/restore" >/dev/null
  check "mongodb restore succeeds" "$(status)" 204
  check "mongodb restore brings back the backed-up documents" "$(mongo 'db.t.countDocuments()' | tail -1)" 1
  call DELETE "/api/v1/databases/$MDB" >/dev/null
  check "mongodb deleted" "$(status)" 204
fi

echo "== volumes"
call POST /api/v1/apps "{\"project_id\":\"$PROJECT\",\"name\":\"Bad Vol\",\"source_kind\":\"image\",\"image\":\"nginx:alpine\",\"port\":80,\"volumes\":[\"/var/run/docker.sock\"]}" >/dev/null
check "engine socket refused as a volume" "$(status)" 422
VOL_APP=$(call POST /api/v1/apps "{\"project_id\":\"$PROJECT\",\"name\":\"Stateful\",\"source_kind\":\"image\",\"image\":\"nginx:alpine\",\"port\":80,\"health_path\":\"/\",\"volumes\":[\"/data/\"],\"env\":[{\"key\":\"MODE\",\"value\":\"demo\",\"is_secret\":false}]}" | jq -r .id)
check "app with a volume created" "$(status)" 201
check "volume path normalised" "$(jq -c .volumes "$WORK/body")" '["/data"]'
check "env set at creation" "$(call GET "/api/v1/apps/$VOL_APP/env" | jq -r '.[0].key')" MODE
V1=$(call POST "/api/v1/apps/$VOL_APP/deploy" | jq -r .id)
wait_for 90 "stateful deployment running" deployment_status_is "$V1" running
docker exec "$(docker ps -q -f "label=rig.app=$VOL_APP")" sh -c 'echo kept > /data/marker'
V2=$(call POST "/api/v1/apps/$VOL_APP/deploy" | jq -r .id)
wait_for 90 "stateful redeploy running" deployment_status_is "$V2" running
check "only one container after a stateful redeploy" "$(docker ps -q -f "label=rig.app=$VOL_APP" | wc -l)" 1
check "apps get the default memory ceiling" "$(docker inspect -f '{{.HostConfig.Memory}}' "$(docker ps -q -f "label=rig.app=$VOL_APP")")" 536870912
check "data survives a redeploy" "$(docker exec "$(docker ps -q -f "label=rig.app=$VOL_APP")" cat /data/marker)" kept

echo "== crash watcher"
check "no crash before one happens" "$(call GET "/api/v1/apps/$VOL_APP" | jq -r .last_crash)" null
VOL_C=$(docker ps -q -f "label=rig.app=$VOL_APP")
docker exec "$VOL_C" sh -c 'kill 1'
wait_for 45 "crash recorded" sh -c "curl -s -b '$JAR' '$API/api/v1/apps/$VOL_APP' | jq -e '.last_crash.restarts >= 1' >/dev/null"
check "app still running after a restarted crash" "$(call GET "/api/v1/apps/$VOL_APP" | jq -r .status)" running
docker stop -t 1 "$VOL_C" >/dev/null
wait_for 60 "stopped container marked failed" app_status_is "$VOL_APP" failed
check "failure reason recorded" "$(call GET "/api/v1/apps/$VOL_APP" | jq -r '.last_crash.reason | length > 0')" true
docker start "$VOL_C" >/dev/null
VOL_NAME=$(docker inspect -f '{{range .Mounts}}{{.Name}}{{end}}' "$(docker ps -q -f "label=rig.app=$VOL_APP")")
call DELETE "/api/v1/apps/$VOL_APP" >/dev/null; check "stateful app deleted" "$(status)" 204
check "its volume is removed" "$(docker volume ls -q -f "name=^${VOL_NAME}$" | wc -l)" 0

echo "== metrics"
wait_for 45 "host metrics sampled" sh -c "curl -s -b '$JAR' '$API/api/v1/system/metrics' | jq -e '.points | length > 0' >/dev/null"
check "app metrics sampled" "$(call GET "/api/v1/apps/$APP/metrics?range=6h" | jq -r '.points | length > 0')" true
check "app metrics report the memory ceiling" "$(jq -r .memory_total "$WORK/body")" 536870912
wait_for 75 "app network throughput sampled" sh -c "curl -s -b '$JAR' '$API/api/v1/apps/$APP/metrics?range=1h' | jq -e '[.points[] | select(.net_rx_bps != null)] | length > 0' >/dev/null"
call GET "/api/v1/apps/$APP/metrics?range=9h" >/dev/null
check "unknown range rejected" "$(status)" 400
check "metrics need sign-in" "$(curl -s -o /dev/null -w '%{http_code}' "$API/api/v1/system/metrics")" 401


if [ -n "${OFFLINE:-}" ]; then
  echo "   skipped (OFFLINE is set)"
else
  call POST /api/v1/apps "{\"project_id\":\"$PROJECT\",\"name\":\"From Git\",\"source_kind\":\"git\",\"git_url\":\"--upload-pack=touch /tmp/pwned\",\"port\":8000}" >/dev/null
  check "hostile git URL rejected" "$(status)" 422
  GIT_APP=$(call POST /api/v1/apps "{\"project_id\":\"$PROJECT\",\"name\":\"From Git\",\"source_kind\":\"git\",\"git_url\":\"${GIT_URL:-https://github.com/crccheck/docker-hello-world}\",\"port\":8000}" | jq -r .id)
  check "git app created" "$(status)" 201
  call POST "/api/v1/apps/$GIT_APP/domains" '{"host":"git.e2e.test","tls_mode":"off"}' >/dev/null
  GIT_DEP=$(call POST "/api/v1/apps/$GIT_APP/deploy" | jq -r .id)
  wait_for 240 "git deployment finished" app_status_is "$GIT_APP" running
  check "git deployment is running" "$(call GET "/api/v1/deployments/$GIT_DEP" | jq -r .status)" running \
    || { jq -r '.error // empty' "$WORK/body"; curl -s -b "$JAR" -N --max-time 3 "$API/api/v1/deployments/$GIT_DEP/logs" | tail -25; }
  check "commit recorded" "$(jq -r '.commit_sha | length' "$WORK/body")" 40
  check "built image served through the proxy" "$(via_proxy git.e2e.test)" 200
  check "build directory removed" "$(find "$WORK/data/builds" -mindepth 1 -maxdepth 1 2>/dev/null | wc -l)" 0

  echo "== railpack build (no Dockerfile)"
  RP_APP=$(call POST /api/v1/apps "{\"project_id\":\"$PROJECT\",\"name\":\"Railpack\",\"source_kind\":\"git\",\"git_url\":\"${RAILPACK_GIT_URL:-https://github.com/render-examples/express-hello-world}\",\"port\":3000}" | jq -r .id)
  check "railpack app created" "$(status)" 201
  call POST "/api/v1/apps/$RP_APP/domains" '{"host":"rp.e2e.test","tls_mode":"off"}' >/dev/null
  RP_DEP=$(call POST "/api/v1/apps/$RP_APP/deploy" | jq -r .id)
  wait_for 900 "railpack deployment finished" sh -c "curl -s -b '$JAR' '$API/api/v1/deployments/$RP_DEP' | jq -e '.status | test(\"running|failed\")' >/dev/null"
  check "railpack deployment is running" "$(call GET "/api/v1/deployments/$RP_DEP" | jq -r .status)" running
  check "build log names Railpack" "$(curl -s -m 20 -b "$JAR" "$API/api/v1/deployments/$RP_DEP/logs" | grep -c 'building with Railpack')" 1
  check "railpack binary cached" "$(test -x "$WORK/data/bin/railpack" -o -n "$(command -v railpack)" && echo yes)" yes
  check "railpack image served through the proxy" "$(via_proxy rp.e2e.test)" 200
  check "railpack build directory removed" "$(find "$WORK/data/builds" -mindepth 1 -maxdepth 1 2>/dev/null | wc -l)" 0
fi

echo "== github app"
GH_HOOK_SECRET=mock-webhook-secret
gh_push() { # event delivery-id body
  local sig; sig=$(printf '%s' "$3" | openssl dgst -sha256 -hmac "$GH_HOOK_SECRET" -r | cut -d' ' -f1)
  curl -s -o "$WORK/body" -w '%{http_code}' -X POST "$API/api/v1/hooks/github/$ORG" -H "x-github-event: $1" \
    -H "x-github-delivery: $2" -H "x-hub-signature-256: sha256=$sig" -H 'content-type: application/json' --data-raw "$3"; }
gh_head() { git -C "$GH/acme/site.git" rev-parse main; }
ORG=$(call GET /api/v1/auth/me | jq -r .org.id)
check "no GitHub App yet" "$(call GET /api/v1/github | jq -r '.app')" null
MANIFEST=$(call POST /api/v1/github/manifest '{"organization":"acme"}')
check "manifest issued" "$(status)" 200
check "manifest posts to the GitHub host" "$(jq -r .post_url <<<"$MANIFEST" | grep -c "^https://127.0.0.1:$GH_GIT/organizations/acme/settings/apps/new?state=")" 1
check "manifest hook points back here" "$(jq -r .manifest <<<"$MANIFEST" | jq -r .hook_attributes.url)" "$API/api/v1/hooks/github/$ORG"
check "manifest asks for read-only contents" "$(jq -r .manifest <<<"$MANIFEST" | jq -r .default_permissions.contents)" read
call POST /api/v1/github/manifest '{"organization":"-bad"}' >/dev/null; check "bad organization refused" "$(status)" 422
GH_STATE=$(jq -r .post_url <<<"$MANIFEST" | sed 's/.*state=//')
call POST /api/v1/github/manifest/complete '{"code":"abc123","state":"forged"}' >/dev/null
check "forged state refused" "$(status)" 422
call POST /api/v1/github/manifest/complete "{\"code\":\"abc123\",\"state\":\"$GH_STATE\"}" >/dev/null
check "manifest completed" "$(status)" 200
check "App is connected" "$(call GET /api/v1/github | jq -r .app.slug)" rig-e2e
call POST /api/v1/github/manifest/complete "{\"code\":\"abc123\",\"state\":\"$GH_STATE\"}" >/dev/null
check "state is single use" "$(status)" 422
check "secrets are not stored in the clear" "$(sqlite3 "$WORK/data/rig.db" "SELECT count(*) FROM github_apps WHERE webhook_secret LIKE '%mock-webhook%' OR private_key LIKE '%PRIVATE KEY%'")" 0
check "installed repositories listed" "$(call GET /api/v1/github/repos | jq -r '.[0].full_name')" acme/site
GH_APP=$(call POST /api/v1/apps "{\"project_id\":\"$PROJECT\",\"name\":\"Site\",\"source_kind\":\"git\",\"git_url\":\"https://127.0.0.1:$GH_GIT/acme/site.git\",\"port\":80}" | jq -r .id)
check "private repo app created" "$(status)" 201
GH_DEP=$(call POST "/api/v1/apps/$GH_APP/deploy" | jq -r .id)
wait_for 240 "private repo deployed" sh -c "curl -s -b '$JAR' '$API/api/v1/deployments/$GH_DEP' | jq -e '.status | test(\"running|failed\")' >/dev/null"
check "private repo cloned with the App's token" "$(call GET "/api/v1/deployments/$GH_DEP" | jq -r .status)" running
[ "$(call GET "/api/v1/deployments/$GH_DEP" | jq -r .status)" = running ] || { call GET "/api/v1/deployments/$GH_DEP" | jq -r .error; curl -s -m 20 -b "$JAR" "$API/api/v1/deployments/$GH_DEP/logs" | tail -15; tail -20 "$GH/requests.log"; }
check "deployed the head commit" "$(call GET "/api/v1/deployments/$GH_DEP" | jq -r .commit_sha)" "$(gh_head)"
check "token never reaches the logs" "$(curl -s -m 20 -b "$JAR" "$API/api/v1/deployments/$GH_DEP/logs" | grep -c ghs_; grep -c ghs_ "$WORK/err.log" "$WORK/out.log" | awk -F: '{s+=$2} END {print s}')" "$(printf '0\n0')"
check "git host saw authorized fetches" "$(grep -c 'git GET .* authorized=True' "$GH/requests.log" | awk '{print ($1>0)?"yes":"no"}')" yes
call PATCH "/api/v1/apps/$GH_APP" '{"auto_deploy":true}' >/dev/null; check "deploy on push enabled" "$(status)" 200
call PATCH "/api/v1/apps/$APP" '{"auto_deploy":true}' >/dev/null; check "deploy on push needs a Git app" "$(status)" 422
GH_EVENT=$(jq -nc --arg a "$(gh_head)" '{ref:"refs/heads/main",after:$a,deleted:false,repository:{full_name:"acme/site",default_branch:"main"}}')
check "unsigned push refused" "$(curl -s -o /dev/null -w '%{http_code}' -X POST "$API/api/v1/hooks/github/$ORG" -H 'x-github-event: push' --data-raw "$GH_EVENT")" 401
check "ping is accepted and skipped" "$(gh_push ping d-ping '{}')" 200
check "push of the deployed commit is skipped" "$(gh_push push d-0 "$GH_EVENT")" 200
gh_commit two
GH_EVENT=$(jq -nc --arg a "$(gh_head)" '{ref:"refs/heads/main",after:$a,deleted:false,repository:{full_name:"acme/site",default_branch:"main"}}')
check "push of a new commit deploys" "$(gh_push push d-1 "$GH_EVENT")" 202
GH_DEP2=$(jq -r '.deployment_ids[0]' "$WORK/body")
check "redelivery is ignored" "$(gh_push push d-1 "$GH_EVENT"; jq -r .skipped "$WORK/body")" "200this delivery was already handled"
check "other branches are ignored" "$(gh_push push d-2 "$(jq -c '.ref="refs/heads/dev"' <<<"$GH_EVENT")"; jq -r '.deployment_ids | length' "$WORK/body")" 2000
wait_for 240 "push deployment finished" sh -c "curl -s -b '$JAR' '$API/api/v1/deployments/$GH_DEP2' | jq -e '.status | test(\"running|failed\")' >/dev/null"
check "push deployment is running" "$(call GET "/api/v1/deployments/$GH_DEP2" | jq -r '.status + " " + .trigger')" "running github"
gh_commit three
wait_for 90 "poller noticed the new commit" sh -c "curl -s -b '$JAR' '$API/api/v1/apps/$GH_APP/deployments' | jq -e '.[0].trigger == \"poll\"' >/dev/null"
wait_for 240 "poll deployment finished" sh -c "curl -s -b '$JAR' '$API/api/v1/apps/$GH_APP/deployments' | jq -e '.[0].status | test(\"running|failed\")' >/dev/null"
# The commit is recorded once the clone is done, not when the deployment is queued.
check "poller deployed the new head" "$(call GET "/api/v1/apps/$GH_APP/deployments" | jq -r '.[0].commit_sha')" "$(gh_head)"
check "poll deployment is running" "$(call GET "/api/v1/apps/$GH_APP/deployments" | jq -r '.[0].status')" running
call DELETE /api/v1/github >/dev/null; check "App disconnected" "$(status)" 204
check "hook refuses once disconnected" "$(gh_push push d-3 "$GH_EVENT")" 401

echo "== cli and mcp"
CLI_BIN=./target/debug/rig
CLI_CFG="$WORK/cli-config.toml"
CLI_TOK_JSON=$(call POST /api/v1/tokens '{"name":"e2e-cli","role":"owner"}')
check "CLI API token created" "$(status)" 200
CLI_TOK_ID=$(jq -r .token.id <<<"$CLI_TOK_JSON")
CLI_TOK=$(jq -r .secret <<<"$CLI_TOK_JSON")
RIG_CONFIG="$CLI_CFG" RIG_TOKEN="$CLI_TOK" "$CLI_BIN" --json login "$API" --name e2e >"$WORK/cli-out.json"
check "cli login succeeds" "$(jq -r .context "$WORK/cli-out.json")" e2e
check "cli config is private" "$(stat -c %a "$CLI_CFG")" 600
check "cli status returns org" "$(RIG_CONFIG="$CLI_CFG" "$CLI_BIN" --json status | jq -r .me.org.name)" Acme
check "cli projects lists Shop" "$(RIG_CONFIG="$CLI_CFG" "$CLI_BIN" --json projects | jq -r '[.[].name] | index("Shop") != null')" true
check "cli apps --project filters server-side" "$(RIG_CONFIG="$CLI_CFG" "$CLI_BIN" --json apps --project Shop | jq -r '[.[].name] | index("Web") != null')" true
check "cli apps show works" "$(RIG_CONFIG="$CLI_CFG" "$CLI_BIN" --json apps show Shop/Web | jq -r .id)" "$APP"
check "cli deployments show works" "$(RIG_CONFIG="$CLI_CFG" "$CLI_BIN" --json deployments show "$D1" | jq -r .id)" "$D1"
check "cli deployments logs emits NDJSON" "$(RIG_CONFIG="$CLI_CFG" "$CLI_BIN" --json deployments logs "$D1" | head -1 | jq -r 'has("line")')" true
check "cli env set works" "$(RIG_CONFIG="$CLI_CFG" "$CLI_BIN" --json env set Shop/Web CLI_VAR=hello --plain | jq -r '.set[0].key')" CLI_VAR
check "cli env get reveals value" "$(RIG_CONFIG="$CLI_CFG" "$CLI_BIN" --json env get Shop/Web CLI_VAR | jq -r .value)" hello
printf 'BULK_A=one\nBULK_B=two\n' >"$WORK/bulk.env"
check "cli env import bulk works" "$(RIG_CONFIG="$CLI_CFG" "$CLI_BIN" --json env import Shop/Web "$WORK/bulk.env" --plain | jq -r .imported)" 2
check "cli env rm deletes key" "$(RIG_CONFIG="$CLI_CFG" "$CLI_BIN" --json env rm Shop/Web CLI_VAR | jq -r .deleted)" CLI_VAR
check "cli domains ls works" "$(RIG_CONFIG="$CLI_CFG" "$CLI_BIN" --json domains ls Shop/Web | jq -r 'length >= 1')" true
check "cli metrics works" "$(RIG_CONFIG="$CLI_CFG" "$CLI_BIN" --json metrics --range 1h | jq -r .range)" 1h
check "cli logs --no-follow works" "$(RIG_CONFIG="$CLI_CFG" "$CLI_BIN" logs Shop/Web --tail 5 --no-follow >/dev/null && echo ok)" ok
check "cli context current works" "$(RIG_CONFIG="$CLI_CFG" "$CLI_BIN" context current)" e2e
MCP_OUT=$(printf '%s\n%s\n%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"system_status","arguments":{}}}' \
  '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_domains","arguments":{"app":"Shop/Web"}}}' \
  | RIG_CONFIG="$CLI_CFG" "$CLI_BIN" mcp)
check "cli mcp initialize succeeds" "$(printf '%s\n' "$MCP_OUT" | sed -n '1p' | jq -r .result.serverInfo.name)" rig
check "cli mcp system_status returns structuredContent" "$(printf '%s\n' "$MCP_OUT" | sed -n '2p' | jq -r .result.structuredContent.organization.name)" Acme
check "cli mcp list_domains works" "$(printf '%s\n' "$MCP_OUT" | sed -n '3p' | jq -r .result.isError)" false
check "cli logout --revoke removes context and token" "$(RIG_CONFIG="$CLI_CFG" "$CLI_BIN" --json logout e2e --revoke | jq -r .revoked)" true

echo "== stop and clean up"
call POST "/api/v1/apps/$APP/stop" >/dev/null; check "app stops" "$(status)" 200
check "stopped app is not routed" "$(docker ps -q -f "label=rig.app=$APP" | wc -l)" 0
call DELETE "/api/v1/projects/$PROJECT" >/dev/null; check "project with a database needs a choice" "$(status)" 409
call DELETE "/api/v1/projects/$PROJECT?databases=delete" >/dev/null; check "project deleted" "$(status)" 204
check "no containers left" "$(docker ps -aq -f "label=rig.app=$APP" -f "label=rig.database=$DB" | wc -l)" 0
check "no network left" "$(docker network ls -q -f "name=^${NET}$" | wc -l)" 0
check "audit log recorded the work" "$(call GET /api/v1/audit | jq '[.[] | select(.action == "app.deploy")] | length > 0')" true
echo "   server memory: $(awk '/VmRSS/ {print $2, $3}' "/proc/$SERVER_PID/status") (debug build)"

echo
if [ "$fails" -eq 0 ]; then echo "all checks passed"; else echo "$fails check(s) failed"; exit 1; fi
