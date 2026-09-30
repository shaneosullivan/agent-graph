import {deleteAccountRecords, stripeCustomerOf} from "@/lib/accounts";
import {clearedCookies, currentUser, sameOrigin, signedInAt} from "@/lib/auth";
import {deleteLogsOf} from "@/lib/cleanup";
import {auth} from "@/lib/firebase";
import {deleteCustomer} from "@/lib/stripe";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * How recently whoever deletes an account must have signed in: a browser
 * left logged in isn't enough.
 */
const SIGNED_IN_WITHIN_MS = 10 * 60 * 1000;

/**
 * Deletes the logged-in account, and everything with it, for good: DELETE
 * /api/account { confirm }, from the account page, where `confirm` is the
 * account's email address (or "delete", for one without), typed out.
 *
 * In this order, so it stops before anything's gone if it can't finish
 * what matters most:
 *
 * 1. its billing ends: its Stripe customer is deleted, which cancels any
 *    subscription at once, with no refund (Stripe keeps the record of what
 *    was paid);
 * 2. its live shares are deleted;
 * 3. its computers' logins, and its account document, go;
 * 4. its login (Firebase Authentication) is deleted, and this browser's
 *    logged out.
 *
 * Logs pasted or uploaded aren't an account's: they stay till they expire.
 * 204. 401 if not logged in, or not signed in the last few minutes (the
 * account page asks to log in again); 400 if `confirm` doesn't match.
 */
export async function DELETE(req: Request): Promise<Response> {
  if (!sameOrigin(req)) {
    return new Response("Forbidden.", {status: 403});
  }
  const user = await currentUser();
  if (!user) {
    return new Response("Log in first.", {status: 401});
  }
  const at = await signedInAt();
  if (at === null || Date.now() - at > SIGNED_IN_WITHIN_MS) {
    return Response.json(
      {signInAgain: true},
      {status: 401, headers: {"Cache-Control": "no-store"}},
    );
  }
  let confirm: unknown;
  try {
    ({confirm} = await req.json());
  } catch {
    return new Response("Expected JSON: { confirm }.", {status: 400});
  }
  const expected = (user.email ?? "delete").trim().toLowerCase();
  if (
    typeof confirm !== "string" ||
    confirm.trim().toLowerCase() !== expected
  ) {
    return new Response(
      user.email
        ? "Type your email address exactly, to confirm."
        : "Type “delete”, to confirm.",
      {status: 400},
    );
  }

  const customer = await stripeCustomerOf(user.uid);
  if (customer) {
    try {
      await deleteCustomer(customer);
    } catch (err) {
      console.error(`Deleting account ${user.uid}: ending billing failed`, err);
      return new Response(
        "Your subscription couldn't be cancelled just now, so nothing's been deleted. Try again in a minute.",
        {status: 502},
      );
    }
  }
  await deleteLogsOf(user.uid);
  await deleteAccountRecords(user.uid);
  try {
    await auth().deleteUser(user.uid);
  } catch (err) {
    // Gone already is fine.
    if ((err as {code?: unknown}).code !== "auth/user-not-found") {
      throw err;
    }
  }

  const headers = new Headers({"Cache-Control": "no-store"});
  for (const cookie of clearedCookies(req)) {
    headers.append("Set-Cookie", cookie);
  }
  return new Response(null, {status: 204, headers});
}
