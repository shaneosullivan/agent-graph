import {webhook} from "@/lib/stripe";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * Stripe's webhook in test mode (a sandbox): POST /api/stripe/webhook-test,
 * signed with STRIPE_TEST_WEBHOOK_SECRET. Live mode's is
 * /api/stripe/webhook. What it does, and replies: `webhook` in
 * lib/stripe.ts.
 */
export function POST(req: Request): Promise<Response> {
  return webhook(req, "test");
}
