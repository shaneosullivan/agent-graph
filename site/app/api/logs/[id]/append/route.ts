import {
  bodyText,
  gone,
  ID_PATTERN,
  MAX_CHUNK_BYTES,
  MAX_LOG_BYTES,
} from "@/lib/config";
import {canWrite} from "@/lib/crypto";
import {
  appendChunk,
  ChunkTaken,
  firstChunkOffset,
  LogGone,
  Misplaced,
} from "@/lib/store";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * Appends a chunk: POST /api/logs/{id}/append?offset={bytes before it}
 *
 * This is the hot path while `agent-graph watch-remote` runs, so it does as
 * little as possible:
 * - the size is checked from the header before the body is read, and as
 *   it's read (it may have none);
 * - so is where it goes: only past `MAX_LOG_BYTES` is the log's first chunk
 *   read, to check it's within that of where the log now starts (a live
 *   share trims its start, though never all of it, nor adds before where it
 *   starts);
 * - the write token is checked by recomputing an HMAC, with no database read;
 * - the body is stored as it arrives (raw JSON Lines, never parsed);
 * - storing it is a single write of a new document keyed by the offset, in
 *   a transaction with the log's metadata (read, that the log's still
 *   there: 410 if it's been deleted, lib/cleanup.ts) and the chunk before
 *   it (read, that this one starts where it ends: 409 if not). Chunks never
 *   overlap, so the span checked above bounds what a log stores.
 *
 * A chunk never changes once stored (viewers don't read one twice): the same
 * bytes again (a retry) are accepted, different ones (or any, where the stored
 * chunk can't be decrypted) refused with 409.
 */
export async function POST(
  req: Request,
  {params}: {params: Promise<{id: string}>},
): Promise<Response> {
  const {id} = await params;
  if (!ID_PATTERN.test(id)) {
    return new Response("Unknown log.", {status: 404});
  }
  if (!canWrite(req, id)) {
    return new Response("Bad or missing write token.", {status: 401});
  }

  const offsetParam = new URL(req.url).searchParams.get("offset") ?? "";
  if (!/^\d{1,15}$/.test(offsetParam)) {
    return new Response("Bad offset.", {status: 400});
  }
  const offset = Number(offsetParam);
  if (offset > MAX_LOG_BYTES) {
    const first = await firstChunkOffset(id);
    if (first === null || offset < first || offset - first > MAX_LOG_BYTES) {
      return new Response("This log is full.", {status: 413});
    }
  }
  if (Number(req.headers.get("content-length") ?? 0) > MAX_CHUNK_BYTES) {
    return new Response(`At most ${MAX_CHUNK_BYTES} bytes per request.`, {
      status: 413,
    });
  }

  const text = await bodyText(req, MAX_CHUNK_BYTES);
  if (text === null) {
    return new Response(`At most ${MAX_CHUNK_BYTES} bytes per request.`, {
      status: 413,
    });
  }
  if (!text) {
    return new Response(null, {status: 204});
  }

  try {
    await appendChunk(id, offset, text);
  } catch (err) {
    if (err instanceof LogGone) {
      return gone();
    }
    if (err instanceof Misplaced) {
      return new Response("That isn't where the log's last chunk ends.", {
        status: 409,
      });
    }
    if (err instanceof ChunkTaken) {
      return new Response("Other events are already stored at this offset.", {
        status: 409,
      });
    }
    throw err;
  }
  return new Response(null, {status: 204});
}
