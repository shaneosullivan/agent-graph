#!/bin/sh
# Starts the built site, waits for it to answer, and runs the API tests,
# then lib/store.ts's own tests.
# Run it inside the Firestore emulator (`npm run test:ci`), which sets
# FIRESTORE_EMULATOR_HOST.
set -e

npx next start -p 3000 &
server=$!
trap 'kill $server 2>/dev/null' EXIT

tries=0
until curl -sf -o /dev/null http://localhost:3000; do
  tries=$((tries + 1))
  if [ "$tries" -ge 60 ]; then
    echo "The site didn't start within a minute." >&2
    exit 1
  fi
  sleep 1
done

BASE_URL=http://localhost:3000 npm run test:api
npm run test:store
