// Where an account stands, and whether Stripe's set up (lib/billing.ts):
// npm run test:unit

import assert from "node:assert/strict";
import {register} from "node:module";
import {test} from "node:test";

register("../scripts/resolve-ts.mjs", import.meta.url);
const {billingConfig, planOf, standing, GRACE_MS} =
  await import("../lib/billing.ts");

const DAY = 24 * 60 * 60 * 1000;
const LABELS = {
  STRIPE_MONTHLY_LABEL: "$5 a month",
  STRIPE_YEARLY_LABEL: "$50 a year",
};
const LIVE = {
  STRIPE_SECRET_KEY: "sk_live_x",
  STRIPE_WEBHOOK_SECRET: "whsec_live",
  STRIPE_PRICE_MONTHLY: "price_live_month",
  STRIPE_PRICE_YEARLY: "price_live_year",
};
const TEST = {
  STRIPE_TEST_SECRET_KEY: "sk_test_x",
  STRIPE_TEST_WEBHOOK_SECRET: "whsec_test",
  STRIPE_TEST_PRICE_MONTHLY: "price_test_month",
  STRIPE_TEST_PRICE_YEARLY: "price_test_year",
};
const ENV = {STRIPE_MODE: "production", ...LIVE, ...TEST, ...LABELS};

test("STRIPE_MODE picks the live or the test keys and prices", () => {
  const live = billingConfig(ENV);
  assert.equal(live?.mode, "production");
  assert.equal(live?.secretKey, "sk_live_x");
  assert.equal(live?.webhookSecret, "whsec_live");
  assert.deepEqual(live?.plans, {
    monthly: {priceId: "price_live_month", label: "$5 a month"},
    yearly: {priceId: "price_live_year", label: "$50 a year"},
  });
  assert.equal(live?.freeDays, 7, "a week free, unless it's set");

  const test = billingConfig({...ENV, STRIPE_MODE: "test"});
  assert.equal(test?.mode, "test");
  assert.equal(test?.secretKey, "sk_test_x");
  assert.equal(test?.webhookSecret, "whsec_test");
  assert.deepEqual(test?.plans, {
    monthly: {priceId: "price_test_month", label: "$5 a month"},
    yearly: {priceId: "price_test_year", label: "$50 a year"},
  });
  assert.equal(planOf(test!, "price_test_year"), "yearly");
  assert.equal(planOf(test!, "price_live_year"), null, "the other mode's");
});

test("Stripe's set up only with every setting its mode needs", () => {
  assert.equal(billingConfig({...LIVE, ...TEST, ...LABELS}), null, "no mode");
  assert.equal(
    billingConfig({...ENV, STRIPE_MODE: "live"}),
    null,
    "not a mode",
  );
  for (const name of [...Object.keys(LIVE), ...Object.keys(LABELS)]) {
    assert.equal(billingConfig({...ENV, [name]: ""}), null, `without ${name}`);
    assert.equal(billingConfig({...ENV, [name]: "  "}), null, `blank ${name}`);
  }
  // Only the mode's own keys and prices are needed.
  assert.ok(billingConfig({STRIPE_MODE: "production", ...LIVE, ...LABELS}));
  assert.ok(billingConfig({STRIPE_MODE: "test", ...TEST, ...LABELS}));
  assert.equal(billingConfig({STRIPE_MODE: "test", ...LIVE, ...LABELS}), null);

  assert.equal(billingConfig({...ENV, FREE_TRIAL_DAYS: "14"})?.freeDays, 14);
  assert.equal(billingConfig({...ENV, FREE_TRIAL_DAYS: "0"})?.freeDays, 0);
  assert.equal(billingConfig({...ENV, FREE_TRIAL_DAYS: "lots"})?.freeDays, 7);
  assert.equal(billingConfig({...ENV, FREE_TRIAL_DAYS: "-1"})?.freeDays, 7);
});

test("without Stripe, every account's active, for good", () => {
  const free = {status: "active", canShare: true, freeUntil: null, until: null};
  assert.deepEqual(standing(null, null, 0), free);
  assert.deepEqual(
    standing(null, {status: "unpaid", createdAt: 0}, 99 * DAY),
    free,
  );
});

test("an unpaid account shares for its free days, then not", () => {
  const billing = billingConfig(ENV);
  const made = 1000 * DAY;
  const paying = {status: "unpaid" as const, createdAt: made};
  const early = standing(billing, paying, made + DAY);
  assert.equal(early.status, "unpaid");
  assert.equal(early.canShare, true);
  assert.equal(early.freeUntil, made + 7 * DAY);
  assert.equal(early.until, made + 7 * DAY, "sharing stops when they end");
  const late = standing(billing, paying, made + 7 * DAY);
  assert.equal(late.canShare, false);
  // One made before accounts had a status is unpaid too.
  assert.equal(
    standing(billing, {createdAt: made}, made + 8 * DAY).canShare,
    false,
  );
});

test("a subscribed account shares till a little past its paid period", () => {
  const billing = billingConfig(ENV);
  const paying = {
    status: "active" as const,
    createdAt: 0,
    periodEnd: 100 * DAY,
  };
  const now = standing(billing, paying, 50 * DAY);
  assert.deepEqual(now, {
    status: "active",
    canShare: true,
    freeUntil: null,
    until: 100 * DAY + GRACE_MS,
  });
  // Not renewed (a webhook missed, say): unpaid, and its free days are long gone.
  const after = standing(billing, paying, 100 * DAY + GRACE_MS + 1);
  assert.equal(after.status, "unpaid");
  assert.equal(after.canShare, false);
});
