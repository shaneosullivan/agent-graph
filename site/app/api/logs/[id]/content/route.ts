import { cookies } from "next/headers";

import { ID_PATTERN, viewCookieName } from "@/lib/config";
import { safeEqual, viewToken } from "@/lib/crypto";
import { getMeta, readChunks } from "@/lib/store";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * Reads a log: GET /api/logs/{id}/content?after={last chunk key seen}
 *
 * Returns the chunks after `after`, joined, as JSON Lines. Headers:
 *   X-Last-Chunk: the key to pass as `after` next time (absent if none)
 *   X-More: 1 if there are more chunks to fetch right away
 * Password-protected logs need the cookie set by /unlock.
 */
export async function GET(req: Request, { params }: { params: Promise<{ id: string }> }): Promise<Response> {
  const { id } = await params;
  if (!ID_PATTERN.test(id)) return new Response("Unknown log.", { status: 404 });
  const after = new URL(req.url).searchParams.get("after") ?? "";
  if (after && !/^\d{15}$/.test(after)) return new Response("Bad cursor.", { status: 400 });

  const meta = await getMeta(id);
  if (!meta) return new Response("Unknown log.", { status: 404 });
  if (meta.pw) {
    const cookie = (await cookies()).get(viewCookieName(id))?.value;
    if (!safeEqual(cookie, viewToken(id, meta.pw))) {
      return new Response("This log needs its password.", { status: 401 });
    }
  }

  const { text, last, more } = await readChunks(id, after);
  const headers: Record<string, string> = {
    "Content-Type": "application/x-ndjson; charset=utf-8",
    "Cache-Control": "private, no-store",
  };
  if (last) headers["X-Last-Chunk"] = last;
  if (more) headers["X-More"] = "1";
  return new Response(text, { headers });
}
