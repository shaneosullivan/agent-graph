#!/usr/bin/env bash
# Installs what the checks need beyond cargo, Node 22 and npm: the Rust
# components and the WebAssembly target, and the site's packages.
set -euo pipefail
cd "$(dirname "$0")/.."

if command -v rustup >/dev/null 2>&1; then
  rustup component add clippy rustfmt
  rustup target add wasm32-unknown-unknown
fi
(cd site && npm ci)
