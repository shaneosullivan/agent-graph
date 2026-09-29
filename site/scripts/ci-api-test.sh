#!/bin/sh
# Starts the built site, waits for it to answer, and runs the API tests,
# then lib/store.ts's own tests.
# Run it inside the Firestore emulator (`npm run test:ci`), which sets
# FIRESTORE_EMULATOR_HOST.
set -e

# The cleanup cron refuses to run without its secret.
export CRON_SECRET="${CRON_SECRET:-ci-only-cron-secret}"

# Stripe's settings, in test mode, so paying for live shares is tested
# (tests/accounts.test.mjs). Nothing's sent to Stripe: they only have to be set.
export STRIPE_MODE="${STRIPE_MODE:-test}"
export STRIPE_TEST_SECRET_KEY="${STRIPE_TEST_SECRET_KEY:-sk_test_ci_only}"
export STRIPE_TEST_WEBHOOK_SECRET="${STRIPE_TEST_WEBHOOK_SECRET:-whsec_ci_only}"
export STRIPE_TEST_PRICE_MONTHLY="${STRIPE_TEST_PRICE_MONTHLY:-price_ci_monthly}"
export STRIPE_TEST_PRICE_YEARLY="${STRIPE_TEST_PRICE_YEARLY:-price_ci_yearly}"
# Live mode's keys too, so its webhook is tested (the site stays in test
# mode, so it only acknowledges what it's sent).
export STRIPE_SECRET_KEY="${STRIPE_SECRET_KEY:-sk_live_ci_only}"
export STRIPE_WEBHOOK_SECRET="${STRIPE_WEBHOOK_SECRET:-whsec_ci_live_only}"
export STRIPE_MONTHLY_LABEL="${STRIPE_MONTHLY_LABEL:-\$5 a month}"
export STRIPE_YEARLY_LABEL="${STRIPE_YEARLY_LABEL:-\$50 a year}"
export FREE_TRIAL_DAYS="${FREE_TRIAL_DAYS:-7}"

# An admin, who can see /admin (tests/accounts.test.mjs makes the account).
export ADMIN_EMAILS="${ADMIN_EMAILS:-admin@agent-graph.test}"

# A cancelled run can leave its server running (on Windows, cancelling
# doesn't stop it): stop any, under CI.
node scripts/stop-stale-servers.mjs

# A port nothing's using, never a fixed one (3000 is too common), so
# nothing else can answer the tests in its place.
port="${PORT:-$(node -e 'const s = require("net").createServer().listen(0, "127.0.0.1", () => { console.log(s.address().port); s.close(); })')}"

# next itself, not through npx, so stopping it stops the server.
node node_modules/next/dist/bin/next start -p "$port" &
server=$!
trap 'kill $server 2>/dev/null' EXIT

tries=0
until curl -sf -o /dev/null "http://localhost:$port"; do
  if ! kill -0 "$server" 2>/dev/null; then
    echo "The site stopped before it answered (is port $port in use?)." >&2
    exit 1
  fi
  tries=$((tries + 1))
  if [ "$tries" -ge 60 ]; then
    echo "The site didn't start within a minute." >&2
    exit 1
  fi
  sleep 1
done

BASE_URL="http://localhost:$port" npm run test:api
npm run test:store
