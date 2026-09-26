#!/usr/bin/env bash
# The checks .github/workflows/ci.yml runs, but for the slow ones
# (scripts/ci-slow-checks.sh): the Rust, the WebAssembly and the site's
# unit tests. Run scripts/ci-setup.sh first.
set -euo pipefail
cd "$(dirname "$0")/.."

step() {
  printf '\n==> %s\n' "$*"
  "$@"
}

# Rust.
step cargo fmt --all --check
step cargo clippy --workspace --all-targets -- -D warnings
step cargo test --workspace

# The core must build without the CLI's dependencies.
step cargo build --lib --no-default-features
step cargo build -p agent-graph-wasm --target wasm32-unknown-unknown --profile wasm
# The site's copy of the viewer must match the crate's, and its WebAssembly
# must be built from the Rust as it is now.
step node site/scripts/sync-viewer.mjs
step git diff --exit-code -- site/public/viewer/app.js site/public/viewer/app.css site/lib/viewer-shell.ts
step node site/scripts/sync-viewer.mjs --check-wasm

# The site.
cd site
step npm run typecheck
step npm run test:unit
