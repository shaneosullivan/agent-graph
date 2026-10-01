#!/usr/bin/env bash
# The slow checks: the site's production build, then its API and store tests
# against the built site and the Firestore emulator, and beside them a build
# of every release target (scripts/build-local.sh), handed out on Chofter CI
# (below).
# Needs Java 21 and port 8080 (the site takes a free port); the Firebase CLI is used from PATH,
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

# step, lane: the site's checks and the release builds run side by side,
# since neither needs the other. The builds are mostly cargo on every core,
# and the site's mostly node and Java on one.
source ../scripts/steps.sh
# (&&: a lane runs without set -e, so the tests aren't run on a failed build.)
site_checks() {
  step npm run build && step npm run test:ci
}
lane "site: build + API tests" site_checks
lane "release builds" bash ../scripts/build-local.sh
lanes_wait

# On Chofter CI, hand out the six builds: copied to the run's output
# directory, which the build machine serves to the LAN, and listed by URL
# in artifacts.txt, which the run uploads to GitHub. Named for their
# targets, as a release's are.
if [ -n "${CHOFTER_OUTPUT_DIR:-}" ] && [ -n "${CHOFTER_OUTPUT_URL:-}" ]; then
  printf '\n==> Handing out the builds\n'
  cd ..
  mkdir -p "$CHOFTER_OUTPUT_DIR/dist"
  # From where build-local.sh puts them: a folder per OS, the ARM build
  # named agent-graph and the x86_64 one agent-graph-x86_64.
  built="$(cargo_target_dir)/dist"
  for target in \
    aarch64-apple-darwin x86_64-apple-darwin \
    aarch64-unknown-linux-musl x86_64-unknown-linux-musl \
    aarch64-pc-windows-msvc x86_64-pc-windows-msvc; do
    ext=""
    [[ "$target" == *-windows-* ]] && ext=.exe
    case "$target" in
      *-apple-darwin) os=mac ;;
      *-linux-*) os=linux ;;
      *) os=windows ;;
    esac
    case "$target" in
      aarch64-*) from="$built/$os/agent-graph$ext" ;;
      *) from="$built/$os/agent-graph-x86_64$ext" ;;
    esac
    name="agent-graph-$target$ext"
    cp "$from" "$CHOFTER_OUTPUT_DIR/dist/$name"
    echo "${CHOFTER_OUTPUT_URL}dist/$name" >>"$CHOFTER_OUTPUT_DIR/artifacts.txt"
  done
  cat "$CHOFTER_OUTPUT_DIR/artifacts.txt"
fi
