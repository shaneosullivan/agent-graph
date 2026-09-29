import {currentUser, sameOrigin} from "@/lib/auth";
import {type Plan, PLANS} from "@/lib/billing";
import {siteUrl} from "@/lib/config";
import {checkout} from "@/lib/stripe";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * Subscribes: POST /api/account/subscribe { plan: "monthly" | "yearly" },
 * from the account page, with the session cookie. Replies { url }: Stripe's
 * Checkout page to pay on (lib/stripe.ts). 404 if the site's free
 * (lib/billing.ts).
 */
export async function POST(req: Request): Promise<Response> {
  if (!sameOrigin(req)) {
    return new Response("Forbidden.", {status: 403});
  }
  const user = await currentUser();
  if (!user) {
    return new Response("Log in first.", {status: 401});
  }
  let plan: unknown;
  try {
    ({plan} = await req.json());
  } catch {
    plan = null;
  }
  if (!PLANS.includes(plan as Plan)) {
    return new Response('Expected JSON: { plan: "monthly" | "yearly" }.', {
      status: 400,
    });
  }
  let url: string | null;
  try {
    url = await checkout(user, siteUrl(req), plan as Plan);
  } catch (err) {
    console.error("Stripe:", err);
    return new Response("Couldn't reach Stripe. Try again in a moment.", {
      status: 502,
    });
  }
  if (!url) {
    return new Response("There's nothing to pay for.", {status: 404});
  }
  return Response.json({url}, {headers: {"Cache-Control": "no-store"}});
}
