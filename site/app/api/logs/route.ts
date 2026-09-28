import {
  bodyText,
  clientAddress,
  MAX_CHUNK_BYTES,
  MAX_PASSWORD_BYTES,
  SOURCES,
  type Source,
  siteUrl,
} from "@/lib/config";
import { hashPassword, newId, PasswordTooLong, passwordFromHeader, writeToken } from "@/lib/crypto";
import { IdTaken, createLog, takeScryptRun } from "@/lib/store";
import { tooMany } from "@/lib/unlock";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * Creates a log.
 *
 * Body: the first chunk of JSON Lines, raw (at most MAX_CHUNK_BYTES).
 * Headers:
 *   X-Agent-Graph-Source: watch | paste | upload
 *   X-Agent-Graph-Password: base64url(UTF-8 password), optional (at most
 *   MAX_PASSWORD_BYTES)
 * Reply (201): { id, url, writeToken }. Send further chunks to
 * /api/logs/{id}/append with the write token. 429 (with Retry-After) if the
 * address has run scrypt too often lately (lib/config.ts).
 */
export async function POST(req: Request): Promise<Response> {
  if (Number(req.headers.get("content-length") ?? 0) > MAX_CHUNK_BYTES) {
    return tooLarge();
  }
  const text = await bodyText(req, MAX_CHUNK_BYTES);
  if (text === null) return tooLarge();

  const header = req.headers.get("x-agent-graph-source") as Source | null;
  const source: Source = header && SOURCES.includes(header) ? header : "paste";

  let password: string | null;
  try {
    password = passwordFromHeader(req);
  } catch (err) {
    if (err instanceof PasswordTooLong) {
      return new Response(`A password may be at most ${MAX_PASSWORD_BYTES} bytes.`, { status: 400 });
    }
    return new Response("Malformed X-Agent-Graph-Password header.", { status: 400 });
  }
  // Hashing it is a scrypt run, which is limited per address.
  if (password) {
    const run = await takeScryptRun(clientAddress(req));
    if ("wait" in run) return tooMany("Too many passwords from this address", run.wait);
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
