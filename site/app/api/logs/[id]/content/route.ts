import {cookies} from "next/headers";

import {accountOfRequest} from "@/lib/accounts";
import {currentUser} from "@/lib/auth";
import {ID_PATTERN, viewCookieName} from "@/lib/config";
import {safeEqual, viewToken} from "@/lib/crypto";
import {storageId} from "@/lib/encryption";
import {getMeta, readChunks} from "@/lib/store";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * Reads a log: GET /api/logs/{id}/content?after={last chunk key seen}
 *
 * Returns the chunks after `after`, joined, as JSON Lines, up to about
 * `BYTES_PER_READ` (the viewer pages through the rest). Headers:
 *   X-First-Chunk: the first chunk's key, its offset: where the text starts
 *     (a trimmed log's first chunk isn't at 0; see `trimLog`)
 *   X-Last-Chunk: the key to pass as `after` next time (absent if none)
 *   X-More: 1 if there are more chunks to fetch right away
 * Password-protected logs need the cookie set by /unlock; an account's live
 * share, its owner's session (lib/auth.ts), or one of the owner's CLI or API
 * tokens, as `Authorization: Bearer <token>`.
 */
export async function GET(
  req: Request,
  {params}: {params: Promise<{id: string}>},
): Promise<Response> {
  const {id} = await params;
  if (!ID_PATTERN.test(id)) {
    return new Response("Unknown log.", {status: 404});
  }
  const after = new URL(req.url).searchParams.get("after") ?? "";
  if (after && !/^\d{15}$/.test(after)) {
    return new Response("Bad cursor.", {status: 400});
  }

  const meta = await getMeta(id);
  if (!meta) {
    return new Response("Unknown log.", {status: 404});
  }
  if (meta.owner && (await readerOf(req)) !== meta.owner) {
    return new Response("Log in to the account whose share this is.", {
      status: 401,
    });
  }
  if (meta.pw) {
    const cookie = (await cookies()).get(viewCookieName(id))?.value;
    if (!safeEqual(cookie, viewToken(id, meta.pw))) {
      return new Response("This log needs its password.", {status: 401});
    }
  }

  let chunks: Awaited<ReturnType<typeof readChunks>>;
  try {
    chunks = await readChunks(id, after);
  } catch (err) {
    // Named by where it's stored: the log's id is what lets people read it.
    console.error(`reading logs/${storageId(id)}:`, err);
    return new Response("This log couldn't be read.", {status: 500});
  }
  const {text, first, last, more} = chunks;
  const headers: Record<string, string> = {
    "Content-Type": "application/x-ndjson; charset=utf-8",
    "Cache-Control": "private, no-store",
  };
  if (first) {
    headers["X-First-Chunk"] = first;
  }
  if (last) {
    headers["X-Last-Chunk"] = last;
  }
  if (more) {
    headers["X-More"] = "1";
  }
  return new Response(text, {headers});
}

/** Who's reading: the logged-in account, or the one whose token it sent. */
async function readerOf(req: Request): Promise<string | undefined> {
  return (await currentUser())?.uid ?? (await accountOfRequest(req))?.uid;
}
