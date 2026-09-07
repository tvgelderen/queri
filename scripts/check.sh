#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --all -- --check
cargo clippy --locked --all-targets --features test-support -- -D warnings
cargo test --locked --features test-support
if [[ -n ${QUERI_TEST_POSTGRES:-} ]]; then
  cargo test --locked --features test-support --lib postgres_ -- --include-ignored
  cargo test --locked --test database postgres_ -- --include-ignored
fi
desktop-file-validate packaging/dev.queri.Queri.desktop
