import { ID_PATTERN, isHttps, viewCookieName } from "@/lib/config";
import { verifyPassword, viewToken } from "@/lib/crypto";
import { getMeta } from "@/lib/store";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

const THIRTY_DAYS = 30 * 24 * 60 * 60;

/**
 * Unlocks a password-protected log for this browser.
 * Body: { "password": "…" }. On success, sets an HttpOnly cookie that the
 * page and /content check.
 */
export async function POST(req: Request, { params }: { params: Promise<{ id: string }> }): Promise<Response> {
  const { id } = await params;
  if (!ID_PATTERN.test(id)) return new Response("Unknown log.", { status: 404 });
  const meta = await getMeta(id);
  if (!meta) return new Response("Unknown log.", { status: 404 });
  if (!meta.pw) return new Response(null, { status: 204 });

  let password = "";
  try {
    const body = (await req.json()) as { password?: unknown };
    if (typeof body.password === "string") password = body.password;
  } catch {
    return new Response('Expected JSON: { "password": "…" }.', { status: 400 });
  }
  if (!password || password.length > 1024 || !(await verifyPassword(password, meta.pw))) {
    return new Response("Wrong password.", { status: 401 });
  }

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
