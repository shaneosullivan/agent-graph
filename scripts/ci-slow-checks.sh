#!/usr/bin/env bash
# The slow checks: the site's production build, then its API and store tests
# against the built site and the Firestore emulator, then a build of every
# release target (scripts/build-local.sh), handed out on Chofter CI (below).
# Needs Java 21 and ports 3000 and 8080; the Firebase CLI is used from PATH,
# or fetched with npx.
# Run scripts/ci-setup.sh first.
set -euo pipefail
cd "$(dirname "$0")/../site"

# Throwaway values for tests only, as CI's.
export AGENT_GRAPH_SECRET="${AGENT_GRAPH_SECRET:-ci-only-secret}"
export CRON_SECRET="${CRON_SECRET:-ci-only-cron-secret}"
# 32 bytes of 0x03, base64url.
export AGENT_GRAPH_ENCRYPTION_KEY="${AGENT_GRAPH_ENCRYPTION_KEY:-AwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwM}"
export FIREBASE_PROJECT_ID="${FIREBASE_PROJECT_ID:-demo-agent-graph}"

# test:ci runs `firebase`: from PATH, or else a shim for npx's.
if ! command -v firebase >/dev/null 2>&1; then
  shim="$(mktemp -d)"
  trap 'rm -rf "$shim"' EXIT
  printf '#!/bin/sh\nexec npx --yes firebase-tools "$@"\n' >"$shim/firebase"
  chmod +x "$shim/firebase"
  PATH="$shim:$PATH"
fi

printf '\n==> npm run build\n'
npm run build
printf '\n==> npm run test:ci\n'
npm run test:ci
printf '\n==> scripts/build-local.sh\n'
bash ../scripts/build-local.sh

# On Chofter CI, hand out the six builds: copied to the run's output
# directory, which the build machine serves to the LAN, and listed by URL
# in artifacts.txt, which the run uploads to GitHub. Named for their
# targets, as a release's are.
if [ -n "${CHOFTER_OUTPUT_DIR:-}" ] && [ -n "${CHOFTER_OUTPUT_URL:-}" ]; then
  printf '\n==> Handing out the builds\n'
  cd ..
  mkdir -p "$CHOFTER_OUTPUT_DIR/dist"
  for target in \
    aarch64-apple-darwin x86_64-apple-darwin \
    aarch64-unknown-linux-musl x86_64-unknown-linux-musl \
    aarch64-pc-windows-msvc x86_64-pc-windows-msvc; do
    ext=""
    [[ "$target" == *-windows-* ]] && ext=.exe
    name="agent-graph-$target$ext"
    cp "target/$target/dist/agent-graph$ext" "$CHOFTER_OUTPUT_DIR/dist/$name"
    echo "${CHOFTER_OUTPUT_URL}dist/$name" >>"$CHOFTER_OUTPUT_DIR/artifacts.txt"
  done
  cat "$CHOFTER_OUTPUT_DIR/artifacts.txt"
fi
