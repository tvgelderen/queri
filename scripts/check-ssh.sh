#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
scripts/ssh-fixture/run.sh bash -c '
  set -euo pipefail
  cargo test --locked --features test-support --lib ssh_
  cargo test --locked --test storage
  cargo test --locked --test ssh -- --ignored --nocapture --test-threads=1
'
