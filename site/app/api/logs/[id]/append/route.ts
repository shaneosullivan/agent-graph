import { bodyText, gone, ID_PATTERN, MAX_CHUNK_BYTES, MAX_LOG_BYTES } from "@/lib/config";
import { canWrite } from "@/lib/crypto";
import { appendChunk, ChunkTaken, firstChunkOffset, LogGone } from "@/lib/store";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * Appends a chunk: POST /api/logs/{id}/append?offset={bytes before it}
 *
 * This is the hot path while `agent-graph watch-remote` runs, so it does as
 * little as possible:
 * - the size is checked from the header before the body is read;
 * - so is the log's: only past `MAX_LOG_BYTES` is its first chunk read, since
 *   what's stored is what counts, and a live share trims its start (though
 *   never all of it, nor adds before where it starts);
 * - the write token is checked by recomputing an HMAC, with no database read;
 * - the body is stored as it arrives (raw JSON Lines, never parsed);
 * - storing it is a single write of a new document keyed by the offset,
 *   with one read, that the log's still there (410 if it's been deleted:
 *   lib/cleanup.ts).
 *
 * A chunk never changes once stored (viewers don't read one twice): the same
 * bytes again (a retry) are accepted, different ones (or any, where the stored
 * chunk can't be decrypted) refused with 409.
 */
export async function POST(req: Request, { params }: { params: Promise<{ id: string }> }): Promise<Response> {
  const { id } = await params;
  if (!ID_PATTERN.test(id)) return new Response("Unknown log.", { status: 404 });
  if (!canWrite(req, id)) return new Response("Bad or missing write token.", { status: 401 });

  const offsetParam = new URL(req.url).searchParams.get("offset") ?? "";
  if (!/^\d{1,15}$/.test(offsetParam)) return new Response("Bad offset.", { status: 400 });
  const offset = Number(offsetParam);
  if (offset > MAX_LOG_BYTES) {
    const first = await firstChunkOffset(id);
    if (first === null || offset < first || offset - first > MAX_LOG_BYTES) {
      return new Response("This log is full.", { status: 413 });
    }
  }
  if (Number(req.headers.get("content-length") ?? 0) > MAX_CHUNK_BYTES) {
    return new Response(`At most ${MAX_CHUNK_BYTES} bytes per request.`, { status: 413 });
  }

  const text = await bodyText(req);
  if (!text) return new Response(null, { status: 204 });
  if (Buffer.byteLength(text) > MAX_CHUNK_BYTES) {
    return new Response(`At most ${MAX_CHUNK_BYTES} bytes per request.`, { status: 413 });
  }

  try {
    await appendChunk(id, offset, text);
  } catch (err) {
    if (err instanceof LogGone) return gone();
    if (err instanceof ChunkTaken) {
      return new Response("Other events are already stored at this offset.", { status: 409 });
    }
    throw err;
  }
  return new Response(null, { status: 204 });
}
