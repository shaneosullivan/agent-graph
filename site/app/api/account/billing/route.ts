import {currentUser, sameOrigin} from "@/lib/auth";
import {siteUrl} from "@/lib/config";
import {portal} from "@/lib/stripe";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * Manages the subscription: POST /api/account/billing, from the account
 * page, with the session cookie. Replies { url }: Stripe's customer portal
 * (lib/stripe.ts), to change how it's paid, or cancel. 404 if the account
 * has never subscribed, or the site's free.
 */
export async function POST(req: Request): Promise<Response> {
  if (!sameOrigin(req)) {
    return new Response("Forbidden.", {status: 403});
  }
  const user = await currentUser();
  if (!user) {
    return new Response("Log in first.", {status: 401});
  }
  let url: string | null;
  try {
    url = await portal(user, siteUrl(req));
  } catch (err) {
    console.error("Stripe:", err);
    return new Response("Couldn't reach Stripe. Try again in a moment.", {
      status: 502,
    });
  }
  if (!url) {
    return new Response("There's no subscription to manage.", {status: 404});
  }
  return Response.json({url}, {headers: {"Cache-Control": "no-store"}});
}
