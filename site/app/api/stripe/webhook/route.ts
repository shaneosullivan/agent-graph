import {webhook} from "@/lib/stripe";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * Stripe's webhook in live mode: POST /api/stripe/webhook, signed with
 * STRIPE_WEBHOOK_SECRET. Test mode's is /api/stripe/webhook-test. What it
 * does, and replies: `webhook` in lib/stripe.ts.
 */
export function POST(req: Request): Promise<Response> {
  return webhook(req, "production");
}
