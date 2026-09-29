import {recordLogin} from "@/lib/accounts";
import {isAdmin} from "@/lib/analytics-core";
import {
  clearedCookies,
  RECENT_SIGN_IN_MS,
  SESSION_MS,
  sameOrigin,
  sessionCookies,
} from "@/lib/auth";
import {auth} from "@/lib/firebase";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * Logs this browser in: POST /api/session { idToken }, with the ID token
 * from its Firebase sign-in (app/login). Sets the session cookie
 * (lib/auth.ts), made from the token, if it's from a sign-in in the last few
 * minutes (an old one could have been stolen), and records the login in the
 * account's document (lib/accounts.ts; made the first time). 204.
 */
export async function POST(req: Request): Promise<Response> {
  if (!sameOrigin(req)) {
    return new Response("Forbidden.", {status: 403});
  }
  let idToken: unknown;
  try {
    ({idToken} = await req.json());
  } catch {
    return new Response("Expected JSON: { idToken }.", {status: 400});
  }
  if (typeof idToken !== "string" || idToken.length > 8192) {
    return new Response("Expected JSON: { idToken }.", {status: 400});
  }
  try {
    const claims = await auth().verifyIdToken(idToken, true);
    if (Date.now() - claims.auth_time * 1000 > RECENT_SIGN_IN_MS) {
      return new Response("Sign in again.", {status: 401});
    }
    const session = await auth().createSessionCookie(idToken, {
      expiresIn: SESSION_MS,
    });
    await recordLogin({uid: claims.uid, email: claims.email ?? null});
    const admin = isAdmin({
      email: claims.email ?? null,
      emailVerified: claims.email_verified === true,
    });
    const headers = new Headers({"Cache-Control": "no-store"});
    for (const cookie of sessionCookies(req, session, SESSION_MS, admin)) {
      headers.append("Set-Cookie", cookie);
    }
    return new Response(null, {status: 204, headers});
  } catch {
    return new Response("That sign-in wasn't accepted. Try again.", {
      status: 401,
    });
  }
}

/** Logs this browser out: DELETE /api/session. 204. */
export async function DELETE(req: Request): Promise<Response> {
  if (!sameOrigin(req)) {
    return new Response("Forbidden.", {status: 403});
  }
  const headers = new Headers({"Cache-Control": "no-store"});
  for (const cookie of clearedCookies(req)) {
    headers.append("Set-Cookie", cookie);
  }
  return new Response(null, {status: 204, headers});
}
