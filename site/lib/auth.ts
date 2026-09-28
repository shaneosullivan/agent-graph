import { cookies } from "next/headers";

import { isHttps } from "./config";
import { auth } from "./firebase";

/**
 * Accounts: who's logged in, in a browser.
 *
 * The browser signs in with Firebase Authentication (email and password, or
 * Google: app/login), then trades the ID token for a session cookie
 * (`SESSION_COOKIE`, made by Firebase from it, HttpOnly), which pages and
 * routes on the server verify. The browser's own Firebase sign-in is then
 * dropped: the cookie is the only record of it. A second cookie
 * (`SIGNED_IN_COOKIE`, not HttpOnly, holding nothing) only tells the pages'
 * scripts to show "Account" rather than "Log in".
 *
 * `agent-graph watch-remote` logs in through the browser too, and is given a
 * token of its own (lib/accounts.ts).
 */

/** Firebase's name for a session cookie; Firebase Hosting passes only it on. */
export const SESSION_COOKIE = "__session";
export const SIGNED_IN_COOKIE = "ag_signed_in";

/** How long a session lasts: Firebase allows at most two weeks. */
export const SESSION_MS = 14 * 24 * 60 * 60 * 1000;

/** How recently the browser must have signed in to be given a session. */
export const RECENT_SIGN_IN_MS = 5 * 60 * 1000;

export type User = { uid: string; email: string | null };

/** The account logged in in this request's browser, if any. */
export async function currentUser(): Promise<User | null> {
  const cookie = (await cookies()).get(SESSION_COOKIE)?.value;
  return cookie ? userOfSession(cookie) : null;
}

/** The account a session cookie is for, if it's valid. */
export async function userOfSession(cookie: string): Promise<User | null> {
  try {
    const claims = await auth().verifySessionCookie(cookie);
    return { uid: claims.uid, email: claims.email ?? null };
  } catch {
    return null;
  }
}

/** Set-Cookie values that log this browser in with `session`, for `maxAge` ms. */
export function sessionCookies(req: Request, session: string, maxAge: number): string[] {
  const seconds = Math.floor(maxAge / 1000);
  const secure = isHttps(req) ? ["Secure"] : [];
  return [
    [
      `${SESSION_COOKIE}=${session}`,
      "Path=/",
      `Max-Age=${seconds}`,
      "HttpOnly",
      "SameSite=Lax",
      ...secure,
    ].join("; "),
    [`${SIGNED_IN_COOKIE}=1`, "Path=/", `Max-Age=${seconds}`, "SameSite=Lax", ...secure].join("; "),
  ];
}

/** Set-Cookie values that log this browser out. */
export function clearedCookies(req: Request): string[] {
  const secure = isHttps(req) ? ["Secure"] : [];
  return [SESSION_COOKIE, SIGNED_IN_COOKIE].map((name) =>
    [`${name}=`, "Path=/", "Max-Age=0", "SameSite=Lax", ...secure].join("; "),
  );
}

/**
 * Whether a request that changes something comes from this site's own pages:
 * browsers say where a request comes from (Origin), which another site can't
 * change, so it can't use the cookie behind the user's back.
 */
export function sameOrigin(req: Request): boolean {
  const origin = req.headers.get("origin");
  const host = req.headers.get("x-forwarded-host") ?? req.headers.get("host");
  if (!origin || !host) return false;
  try {
    return new URL(origin).host === host;
  } catch {
    return false;
  }
}
