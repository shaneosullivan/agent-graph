import Stripe from "stripe";

import {
  type Account,
  payingOf,
  setStripeCustomer,
  setSubscription,
  stripeCustomerOf,
} from "./accounts";
import {
  type Billing,
  billingConfig,
  type Plan,
  planOf,
  standing,
} from "./billing";

/**
 * Subscribing, with Stripe (lib/billing.ts says when it's needed):
 *
 * - The account page's Subscribe buttons, monthly and yearly, make a
 *   Checkout Session (`checkout`) for that plan's price, and send the
 *   browser to Stripe's page for it. It lists no payment methods, so Stripe offers
 *   those turned on in its Dashboard: cards, Link, Apple Pay, Google Pay…
 *   Each account has one Stripe customer, made the first time; the
 *   subscription carries the account's uid (`metadata.uid`).
 * - Stripe sends the browser back to /account?checkout={session id}, which
 *   records the subscription straight away (`finishCheckout`). Stripe tells
 *   the webhook (app/api/stripe/webhook) of it too, and of every change
 *   after: renewed, a payment failed, canceled… (`syncSubscription`).
 * - Manage billing opens Stripe's customer portal (`portal`), to change the
 *   card or cancel.
 */

let client: {key: string; stripe: Stripe} | null = null;

/** Stripe's client, with STRIPE_MODE's secret key; null if Stripe isn't set up. */
export function stripe(
  billing: Billing | null = billingConfig(),
): Stripe | null {
  if (!billing) {
    return null;
  }
  if (client?.key !== billing.secretKey) {
    client = {key: billing.secretKey, stripe: new Stripe(billing.secretKey)};
  }
  return client.stripe;
}

/** Account `account`'s Stripe customer, made the first time. */
async function customerOf(s: Stripe, account: Account): Promise<string> {
  const existing = await stripeCustomerOf(account.uid);
  if (existing) {
    return existing;
  }
  const customer = await s.customers.create(
    {
      ...(account.email ? {email: account.email} : {}),
      metadata: {uid: account.uid},
    },
    // Two clicks at once make one customer.
    {idempotencyKey: `customer-${account.uid}`},
  );
  await setStripeCustomer(account.uid, customer.id);
  return customer.id;
}

/** Stripe won't start a trial that ends less than two days away. */
const MIN_TRIAL_MS = 2 * 24 * 60 * 60 * 1000 + 60 * 60 * 1000;

/**
 * A Checkout Session to subscribe `account` to `plan`, returning to
 * `site`/account: the address of Stripe's page for it. The free days still
 * to come are kept, as a trial: nothing's charged till they're over.
 */
export async function checkout(
  account: Account,
  site: string,
  plan: Plan,
): Promise<string | null> {
  const billing = billingConfig();
  const s = stripe(billing);
  if (!billing || !s) {
    return null;
  }
  const customer = await customerOf(s, account);
  const {freeUntil} = standing(
    billing,
    await payingOf(account.uid),
    Date.now(),
  );
  const trialEnd =
    freeUntil !== null && freeUntil - Date.now() > MIN_TRIAL_MS
      ? Math.floor(freeUntil / 1000)
      : undefined;
  const session = await s.checkout.sessions.create({
    mode: "subscription",
    customer,
    client_reference_id: account.uid,
    line_items: [{price: billing.plans[plan].priceId, quantity: 1}],
    subscription_data: {
      metadata: {uid: account.uid},
      ...(trialEnd ? {trial_end: trialEnd} : {}),
    },
    success_url: `${site}/account?checkout={CHECKOUT_SESSION_ID}`,
    cancel_url: `${site}/account`,
  });
  return session.url;
}

/** Stripe's customer portal for `account`, returning to `site`/account. */
export async function portal(
  account: Account,
  site: string,
): Promise<string | null> {
  const s = stripe();
  const customer = await stripeCustomerOf(account.uid);
  if (!s || !customer) {
    return null;
  }
  const session = await s.billingPortal.sessions.create({
    customer,
    return_url: `${site}/account`,
  });
  return session.url;
}

/**
 * Records a subscription as Stripe has it now, for the account in its
 * metadata. Whether it was one of an account's.
 */
async function recordSubscription(
  billing: Billing,
  sub: Stripe.Subscription,
): Promise<boolean> {
  const uid = sub.metadata?.uid;
  if (!uid) {
    return false;
  }
  // The period's end is its items' (they're all the one price).
  const periodEnd = sub.items.data.reduce<number | null>(
    (end, item) =>
      end === null
        ? item.current_period_end
        : Math.min(end, item.current_period_end),
    null,
  );
  await setSubscription(
    uid,
    typeof sub.customer === "string" ? sub.customer : sub.customer.id,
    {
      id: sub.id,
      status: sub.status,
      plan: planOf(billing, sub.items.data[0]?.price.id ?? ""),
      periodEnd: periodEnd === null ? null : periodEnd * 1000,
      cancelAt: sub.cancel_at === null ? null : sub.cancel_at * 1000,
    },
  );
  return true;
}

/**
 * Records subscription `id` as Stripe has it now, not as an event said it
 * was: Stripe's events can come out of order, or more than once.
 */
export async function syncSubscription(id: string): Promise<boolean> {
  const billing = billingConfig();
  const s = stripe(billing);
  return billing && s
    ? recordSubscription(billing, await s.subscriptions.retrieve(id))
    : false;
}

/**
 * Records the subscription a Checkout Session of `uid`'s made, as its
 * browser comes back from Stripe, rather than waiting for the webhook.
 */
export async function finishCheckout(uid: string, id: string): Promise<void> {
  const billing = billingConfig();
  const s = stripe(billing);
  if (!billing || !s || !/^cs_[A-Za-z0-9_]{1,200}$/.test(id)) {
    return;
  }
  const session = await s.checkout.sessions.retrieve(id, {
    expand: ["subscription"],
  });
  if (
    session.client_reference_id === uid &&
    session.subscription &&
    typeof session.subscription !== "string"
  ) {
    await recordSubscription(billing, session.subscription);
  }
}
