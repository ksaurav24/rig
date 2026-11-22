#!/usr/bin/env bash
# Start a local Rig for development: a Caddy container on high ports, the
# server on 127.0.0.1:7460, and the Vite dev server on 5199.
#
#   scripts/dev.sh          start (or restart) everything
#   scripts/dev.sh stop     stop everything
#
# State lives in .dev/ (ignored by git). The one-time setup token is printed
# on first start and kept in .dev/token.txt.
set -euo pipefail
cd "$(dirname "$0")/.."

DEV="$PWD/.dev"
CADDY=rig-caddy-dev
ADMIN=127.0.0.1:27019
mkdir -p "$DEV"

stop() {
  for name in server vite; do
    if [ -f "$DEV/$name.pid" ]; then
      kill "$(cat "$DEV/$name.pid")" 2>/dev/null || true
      rm -f "$DEV/$name.pid"
    fi
  done
}

stop
if [ "${1:-}" = stop ]; then
  docker rm -f "$CADDY" >/dev/null 2>&1 || true
  exit 0
fi

if [ -z "$(docker ps -q -f "name=^${CADDY}$")" ]; then
  docker rm -f "$CADDY" >/dev/null 2>&1 || true
  printf '{"admin":{"listen":"%s"}}' "$ADMIN" > "$DEV/caddy.json"
  TS_MOUNT=(); [ -S /run/tailscale/tailscaled.sock ] && TS_MOUNT=(-v /run/tailscale:/run/tailscale)
  docker run -d --name "$CADDY" --network host "${TS_MOUNT[@]}" \
    -v "$DEV/caddy.json:/etc/caddy/caddy.json:ro" \
    caddy:2-alpine caddy run --config /etc/caddy/caddy.json >/dev/null
fi

cargo build -q -p rig-server
setsid nohup ./target/debug/rig-server --data-dir "$DEV/data" \
  --caddy-admin "http://$ADMIN" --http-port 28080 --https-port 28443 \
  </dev/null >"$DEV/server.out" 2>"$DEV/server.err" &
echo $! > "$DEV/server.pid"

(cd web && setsid nohup pnpm exec vite --port 5199 --strictPort </dev/null >"$DEV/vite.log" 2>&1 &
 echo $! > "$DEV/vite.pid")

sleep 1
token=$(grep -oE '^ {6}\S+' "$DEV/server.err" | tr -d ' ' || true)
[ -n "$token" ] && echo "$token" > "$DEV/token.txt"
echo "dashboard  http://localhost:5199"
echo "api        http://127.0.0.1:7460/api/docs"
[ -f "$DEV/token.txt" ] && echo "setup token (if setup is not done yet): $(cat "$DEV/token.txt")"
