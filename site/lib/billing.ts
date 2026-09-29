/**
 * Paying for live shares: what's configured, and where an account stands.
 *
 * With Stripe set up (`STRIPE_MODE`, and every setting it needs:
 * `stripeEnv`), an account is made `unpaid`, and can share live anyway for
 * its first `FREE_TRIAL_DAYS` days (7 if unset); after that, only once it's
 * subscribed (`active`), monthly or yearly, from its account page (Stripe
 * Checkout: lib/stripe.ts). Without it (any of them unset), the site's
 * free: every account is `active`.
 *
 * `STRIPE_MODE` is `test` or `production`, and picks which keys and prices
 * are used: Stripe's test mode and live mode each have their own, and one
 * doesn't work with the other's. The prices' labels are the same in both.
 *
 * Starting a share (or carrying on with one) checks where the account
 * stands, and stamps the log with when sharing to it stops (`until`): its
 * appends are refused after that, with 402, and the CLI starts the share
 * again, which checks again. So a subscription that's renewed carries on
 * without a break, and one that isn't stops being shared to, with no
 * database read on the append path (lib/store.ts reads the log anyway).
 *
 * No imports: the unit tests load it directly.
 */

/** Where an account stands. The CLI is told it (src/account.rs). */
export type AccountStatus = "unpaid" | "active";

export type StripeMode = "test" | "production";

/** How often a subscription's paid for. */
export type Plan = "monthly" | "yearly";
export const PLANS: ReadonlyArray<Plan> = ["monthly", "yearly"];

/**
 * The environment variables Stripe needs in `mode`, besides STRIPE_MODE:
 * its own keys and prices (`STRIPE_TEST_…` in test mode), and the labels.
 */
export function stripeEnv(mode: StripeMode) {
  const prefix = mode === "test" ? "STRIPE_TEST_" : "STRIPE_";
  return {
    secretKey: `${prefix}SECRET_KEY`,
    webhookSecret: `${prefix}WEBHOOK_SECRET`,
    monthlyPrice: `${prefix}PRICE_MONTHLY`,
    yearlyPrice: `${prefix}PRICE_YEARLY`,
    monthlyLabel: "STRIPE_MONTHLY_LABEL",
    yearlyLabel: "STRIPE_YEARLY_LABEL",
  } as const;
}

export type Billing = {
  mode: StripeMode;
  /** Stripe's secret API key (sk_live_… or sk_test_…), or a restricted one. */
  secretKey: string;
  /** The webhook endpoint's signing secret (whsec_…). */
  webhookSecret: string;
  /** Each plan's recurring price (price_…), and its cost as people are shown it: "$5 a month", say. */
  plans: Record<Plan, {priceId: string; label: string}>;
  /** Days an account can share live before it has to subscribe. */
  freeDays: number;
};

export const DEFAULT_FREE_DAYS = 7;

/**
 * How long past the end of a paid period sharing carries on: Stripe
 * renews a subscription at the period's end, retries a failed payment over
 * the days after, and its webhooks can be late.
 */
export const GRACE_MS = 3 * 24 * 60 * 60 * 1000;

const DAY_MS = 24 * 60 * 60 * 1000;

/** What's wrong with Stripe's settings, said once: the site's free meanwhile. */
let warned = false;
function warn(problem: string) {
  if (!warned) {
    warned = true;
    console.warn(`Stripe isn't set up, so sharing live is free: ${problem}`);
  }
}

/**
 * Stripe's settings, for STRIPE_MODE, if every one it needs is set; null if
 * not, and the site's free. (STRIPE_MODE set, with something missing or
 * wrong, is said once, as a warning.)
 */
export function billingConfig(
  env: Record<string, string | undefined> = process.env,
): Billing | null {
  const get = (name: string) => env[name]?.trim() ?? "";
  const mode = get("STRIPE_MODE");
  if (!mode) {
    return null;
  }
  if (mode !== "test" && mode !== "production") {
    warn(`STRIPE_MODE is "${mode}", not "test" or "production".`);
    return null;
  }
  const names = stripeEnv(mode);
  const missing = Object.values(names).filter(name => !get(name));
  if (missing.length) {
    warn(`STRIPE_MODE is ${mode}, but ${missing.join(", ")} isn't set.`);
    return null;
  }
  const days = Number(get("FREE_TRIAL_DAYS") || DEFAULT_FREE_DAYS);
  return {
    mode,
    secretKey: get(names.secretKey),
    webhookSecret: get(names.webhookSecret),
    plans: {
      monthly: {
        priceId: get(names.monthlyPrice),
        label: get(names.monthlyLabel),
      },
      yearly: {priceId: get(names.yearlyPrice), label: get(names.yearlyLabel)},
    },
    freeDays: Number.isFinite(days) && days >= 0 ? days : DEFAULT_FREE_DAYS,
  };
}

/** Which plan a price is, if it's one of `billing`'s. */
export function planOf(billing: Billing, priceId: string): Plan | null {
  return PLANS.find(plan => billing.plans[plan].priceId === priceId) ?? null;
}

/** What an account's document says about paying (lib/accounts.ts), in ms. */
export type Paying = {
  status?: AccountStatus;
  createdAt?: number;
  /** When the subscription's paid period ends. */
  periodEnd?: number;
};

export type Standing = {
  status: AccountStatus;
  /** Whether it can share live now. */
  canShare: boolean;
  /** When its free days end, while it's unpaid. */
  freeUntil: number | null;
  /** When sharing stops, unless it's checked again first; null: never. */
  until: number | null;
};

/** Where an account stands at `now`. */
export function standing(
  billing: Billing | null,
  paying: Paying | null,
  now: number,
): Standing {
  if (!billing) {
    return {status: "active", canShare: true, freeUntil: null, until: null};
  }
  if (paying?.status === "active") {
    const until = paying.periodEnd ? paying.periodEnd + GRACE_MS : null;
    // (Past it, a renewal's webhook was missed: it's unpaid, as far as is
    // known, until Stripe says otherwise.)
    if (until === null || now < until) {
      return {status: "active", canShare: true, freeUntil: null, until};
    }
  }
  const freeUntil = (paying?.createdAt ?? now) + billing.freeDays * DAY_MS;
  return {
    status: "unpaid",
    canShare: now < freeUntil,
    freeUntil,
    until: freeUntil,
  };
}

/** What the CLI's told of where an account stands, with a share it starts. */
export function standingReply(s: Standing) {
  return {accountStatus: s.status, freeUntil: s.freeUntil};
}

/**
 * The reply to starting a share (or appending to one) once the account's
 * free days are over and it hasn't subscribed: 402. The CLI opens the
 * account page (`site`/account), to subscribe.
 */
export function mustSubscribe(site: string): Response {
  return new Response(
    `Your free days are over: subscribe to keep sharing live, at ${site}/account`,
    {status: 402, headers: {"Cache-Control": "no-store"}},
  );
}
