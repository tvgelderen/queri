#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ $(uname -s) != Linux ]]; then
  echo "Build this bundle on Linux." >&2
  exit 1
fi

cargo build --locked --release
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
name="queri-${version}-linux-$(uname -m)"
stage=$(mktemp -d "${TMPDIR:-/tmp}/queri-package.XXXXXX")
trap 'rm -rf "$stage"' EXIT
bundle="$stage/$name"
install -Dm755 target/release/queri "$bundle/bin/queri"
install -Dm644 packaging/dev.queri.Queri.desktop "$bundle/share/applications/dev.queri.Queri.desktop"
install -Dm644 packaging/dev.queri.Queri.svg "$bundle/share/icons/hicolor/scalable/apps/dev.queri.Queri.svg"
install -Dm644 README.md "$bundle/README.md"
install -Dm755 scripts/install-linux.sh "$bundle/install.sh"
mkdir -p dist
tar -C "$stage" -czf "dist/$name.tar.gz" "$name"
sha256sum "dist/$name.tar.gz" > "dist/$name.tar.gz.sha256"
echo "Built dist/$name.tar.gz"
