#!/usr/bin/env bash
# Builds a release bundle for Linux servers:
#
#   scripts/package.sh   ->  dist/rig-<version>-x86_64-linux.tar.gz
#
# The binaries are built inside Debian 12 so they link against an old glibc
# and run on every supported distribution (Debian 12+, Ubuntu 24.04+,
# Fedora/RHEL 9+), whatever this machine runs. Needs docker (or podman, with
# ENGINE=podman) and pnpm.
#
# On the server:  tar xzf rig-*.tar.gz && cd rig-*/ && sudo ./install.sh
set -euo pipefail
cd "$(dirname "$0")/.."

version=$(awk -F'"' '/^version/ { print $2; exit }' Cargo.toml)
name=rig-$version-x86_64-linux
builder=${RIG_BUILDER_IMAGE:-docker.io/library/rust:1-bookworm}
ENGINE=${ENGINE:-docker}
# Rootless Podman maps the container's users onto subordinate IDs; keep-id
# makes the build write files as this user, as -u does on Docker.
user_flags=(-u "$(id -u):$(id -g)")
[ "$ENGINE" = podman ] && user_flags=(--userns=keep-id "${user_flags[@]}")

echo "==> building the dashboard"
(cd web && pnpm install --frozen-lockfile --silent && pnpm build >/dev/null)

echo "==> building the binaries in $builder"
mkdir -p target/bookworm/cargo-home
"$ENGINE" run --rm \
  "${user_flags[@]}" \
  -e CARGO_HOME=/cargo -e HOME=/tmp \
  -v "$PWD:/src" -w /src \
  -v "$PWD/target/bookworm/cargo-home:/cargo" \
  "$builder" \
  cargo build --release --locked --target-dir target/bookworm -p rig-server -p rig-cli

out=dist/$name
rm -rf "$out"
mkdir -p "$out"
cp target/bookworm/release/rig-server target/bookworm/release/rig "$out/"
strip "$out/rig-server" "$out/rig" 2>/dev/null || true
cp packaging/install.sh packaging/rig.service "$out/"
cp docs/deploy-on-a-server.md "$out/README.md"
chmod 0755 "$out/install.sh"
cp packaging/install.sh dist/install.sh
chmod 0755 dist/install.sh
tar -C dist -czf "dist/$name.tar.gz" "$name"
(cd dist && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
echo "==> dist/$name.tar.gz ($(du -h "dist/$name.tar.gz" | cut -f1))"
echo "==> dist/install.sh"
