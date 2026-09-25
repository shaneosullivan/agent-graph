import { MAX_CHUNK_BYTES, SOURCES, type Source, siteUrl } from "@/lib/config";
import { hashPassword, newId, passwordFromHeader, writeToken } from "@/lib/crypto";
import { IdTaken, createLog } from "@/lib/store";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * Creates a log.
 *
 * Body: the first chunk of JSON Lines, raw (at most MAX_CHUNK_BYTES).
 * Headers:
 *   X-Agent-Graph-Source: watch | paste | upload
 *   X-Agent-Graph-Password: base64url(UTF-8 password), optional
 * Reply (201): { id, url, writeToken }. Send further chunks to
 * /api/logs/{id}/append with the write token.
 */
export async function POST(req: Request): Promise<Response> {
  if (Number(req.headers.get("content-length") ?? 0) > MAX_CHUNK_BYTES) {
    return tooLarge();
  }
  const text = await req.text();
  if (Buffer.byteLength(text) > MAX_CHUNK_BYTES) return tooLarge();

  const header = req.headers.get("x-agent-graph-source") as Source | null;
  const source: Source = header && SOURCES.includes(header) ? header : "paste";

  let password: string | null;
  try {
    password = passwordFromHeader(req);
  } catch {
    return new Response("Malformed X-Agent-Graph-Password header.", { status: 400 });
  }
  const pw = password ? await hashPassword(password) : undefined;

  for (let attempt = 0; attempt < 3; attempt++) {
    const id = newId();
    try {
      await createLog(id, { source, pw }, text);
    } catch (err) {
      if (err instanceof IdTaken) continue;
      throw err;
    }
    return Response.json(
      { id, url: `${siteUrl(req)}/l/${id}`, writeToken: writeToken(id) },
      { status: 201, headers: { "Cache-Control": "no-store" } },
    );
  }
  return new Response("Couldn't allocate an id; try again.", { status: 503 });
}

function tooLarge(): Response {
  return new Response(`Each request may carry at most ${MAX_CHUNK_BYTES} bytes.`, { status: 413 });
}
