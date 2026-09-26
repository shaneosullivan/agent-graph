import { bodyText, clientAddress, ID_PATTERN, isHttps, UNLOCK_BODY_BYTES, viewCookieName } from "./config";
import { verifyPassword, viewToken } from "./crypto";
import { getMeta, giveBackUnlockAttempt, takeUnlockAttempt } from "./store";

const THIRTY_DAYS = 30 * 24 * 60 * 60;

/** What unlocking uses, passed in so tests can see what's called, and when. */
export type UnlockDeps = {
  getMeta: typeof getMeta;
  takeUnlockAttempt: typeof takeUnlockAttempt;
  giveBackUnlockAttempt: typeof giveBackUnlockAttempt;
  verifyPassword: typeof verifyPassword;
};

const defaults: UnlockDeps = { getMeta, takeUnlockAttempt, giveBackUnlockAttempt, verifyPassword };

/**
 * Unlocks password-protected log `id` for this browser (the unlock route).
 * Body: { "password": "…" }. On success, sets an HttpOnly cookie that the
 * page and /content check.
 *
 * Every guess is counted before the password is checked (a scrypt run), and
 * a right one given back: past the limits on wrong ones (lib/config.ts),
 * it's refused with 429 until the window ends, without checking it.
 */
export async function unlock(req: Request, id: string, deps: UnlockDeps = defaults): Promise<Response> {
  if (!ID_PATTERN.test(id)) return new Response("Unknown log.", { status: 404 });
  const meta = await deps.getMeta(id);
  if (!meta) return new Response("Unknown log.", { status: 404 });
  if (!meta.pw) return new Response(null, { status: 204 });

  const text = await bodyText(req, UNLOCK_BODY_BYTES);
  if (text === null) return new Response("Too long.", { status: 413 });
  let password = "";
  try {
    const body = JSON.parse(text) as { password?: unknown };
    if (typeof body.password === "string") password = body.password;
  } catch {
    return new Response('Expected JSON: { "password": "…" }.', { status: 400 });
  }
  if (!password || password.length > 1024) return new Response("Wrong password.", { status: 401 });

  const attempt = await deps.takeUnlockAttempt(id, clientAddress(req));
  if ("wait" in attempt) {
    const minutes = Math.ceil(attempt.wait / 60);
    const when = attempt.wait < 60 ? "a few seconds" : `${minutes} minute${minutes === 1 ? "" : "s"}`;
    return new Response(`Too many tries. Try again in ${when}.`, {
      status: 429,
      headers: { "Retry-After": String(attempt.wait) },
    });
  }
  if (!(await deps.verifyPassword(password, meta.pw))) return new Response("Wrong password.", { status: 401 });
  await deps.giveBackUnlockAttempt(attempt.reservation);

  const cookie = [
    `${viewCookieName(id)}=${viewToken(id, meta.pw)}`,
    "Path=/",
    `Max-Age=${THIRTY_DAYS}`,
    "HttpOnly",
    "SameSite=Lax",
    ...(isHttps(req) ? ["Secure"] : []),
  ].join("; ");
  return new Response(null, { status: 204, headers: { "Set-Cookie": cookie } });
}
