import { ID_PATTERN, MAX_CHUNK_BYTES, MAX_LOG_BYTES } from "@/lib/config";
import { canWrite } from "@/lib/crypto";
import { appendChunk } from "@/lib/store";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * Appends a chunk: POST /api/logs/{id}/append?offset={bytes before it}
 *
 * This is the hot path while `agent-graph watch-remote` runs, so it does as
 * little as possible:
 * - the size is checked from the header before the body is read;
 * - the write token is checked by recomputing an HMAC, with no database read;
 * - the body is stored as it arrives (raw JSON Lines, never parsed);
 * - storing it is a single write of a new document keyed by the offset.
 */
export async function POST(req: Request, { params }: { params: Promise<{ id: string }> }): Promise<Response> {
  const { id } = await params;
  if (!ID_PATTERN.test(id)) return new Response("Unknown log.", { status: 404 });
  if (!canWrite(req, id)) return new Response("Bad or missing write token.", { status: 401 });

  const offsetParam = new URL(req.url).searchParams.get("offset") ?? "";
  if (!/^\d{1,15}$/.test(offsetParam)) return new Response("Bad offset.", { status: 400 });
  const offset = Number(offsetParam);
  if (offset > MAX_LOG_BYTES) return new Response("This log is full.", { status: 413 });
  if (Number(req.headers.get("content-length") ?? 0) > MAX_CHUNK_BYTES) {
    return new Response(`At most ${MAX_CHUNK_BYTES} bytes per request.`, { status: 413 });
  }

  const text = await req.text();
  if (!text) return new Response(null, { status: 204 });
  if (Buffer.byteLength(text) > MAX_CHUNK_BYTES) {
    return new Response(`At most ${MAX_CHUNK_BYTES} bytes per request.`, { status: 413 });
  }

  await appendChunk(id, offset, text);
  return new Response(null, { status: 204 });
}
