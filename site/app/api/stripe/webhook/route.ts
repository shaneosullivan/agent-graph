import type Stripe from "stripe";

import {billingConfig} from "@/lib/billing";
import {stripe, syncSubscription} from "@/lib/stripe";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/** The events it's sent (set on the endpoint in Stripe's Dashboard). */
const SUBSCRIPTION_EVENTS = new Set([
  "customer.subscription.created",
  "customer.subscription.updated",
  "customer.subscription.deleted",
  "customer.subscription.paused",
  "customer.subscription.resumed",
]);

/**
 * Stripe's webhook: POST /api/stripe/webhook, signed with the endpoint's
 * secret (STRIPE_WEBHOOK_SECRET). A subscription's made, renewed, fails to
 * be paid for, or ends, and the account's brought up to date
 * (lib/stripe.ts): `checkout.session.completed` and the
 * `customer.subscription.*` events. Others are acknowledged, and ignored.
 * 200 once it's recorded; anything else, and Stripe sends it again.
 */
export async function POST(req: Request): Promise<Response> {
  const billing = billingConfig();
  const s = stripe(billing);
  if (!billing || !s) {
    return new Response("Stripe isn't set up here.", {status: 404});
  }
  const signature = req.headers.get("stripe-signature");
  if (!signature) {
    return new Response("Missing Stripe-Signature.", {status: 400});
  }
  let event: Stripe.Event;
  try {
    // The body as sent: the signature is of its bytes.
    event = await s.webhooks.constructEventAsync(
      await req.text(),
      signature,
      billing.webhookSecret,
    );
  } catch {
    return new Response("Bad signature.", {status: 400});
  }

  let subscription: string | null = null;
  if (event.type === "checkout.session.completed") {
    const sub = event.data.object.subscription;
    subscription = typeof sub === "string" ? sub : (sub?.id ?? null);
  } else if (SUBSCRIPTION_EVENTS.has(event.type)) {
    subscription = (event.data.object as Stripe.Subscription).id;
  }
  if (subscription) {
    await syncSubscription(subscription);
  }
  return Response.json({received: true});
}
