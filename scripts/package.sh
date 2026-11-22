#!/usr/bin/env bash
# Builds a release bundle for Linux servers:
#
#   scripts/package.sh   ->  dist/rig-<version>-x86_64-linux.tar.gz
#
# The binaries are built inside Debian 12 so they link against an old glibc
# and run on every supported distribution (Debian 12+, Ubuntu 24.04+,
# Fedora/RHEL 9+), whatever this machine runs. Needs docker and pnpm.
#
# On the server:  tar xzf rig-*.tar.gz && cd rig-*/ && sudo ./install.sh
set -euo pipefail
cd "$(dirname "$0")/.."

version=$(awk -F'"' '/^version/ { print $2; exit }' Cargo.toml)
name=rig-$version-x86_64-linux
builder=${RIG_BUILDER_IMAGE:-rust:1-bookworm}

echo "==> building the dashboard"
(cd web && pnpm install --frozen-lockfile --silent && pnpm build >/dev/null)

echo "==> building the binaries in $builder"
mkdir -p target/bookworm/cargo-home
docker run --rm \
  -u "$(id -u):$(id -g)" \
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
tar -C dist -czf "dist/$name.tar.gz" "$name"
(cd dist && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
echo "==> dist/$name.tar.gz ($(du -h "dist/$name.tar.gz" | cut -f1))"
