#!/usr/bin/env bash
# The slow checks: the site's production build, then its API and store tests
# against the built site and the Firestore emulator, then a build of every
# release target (scripts/build-local.sh). Needs Java 21 and ports 3000 and
# 8080; the Firebase CLI is used from PATH, or fetched with npx.
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
