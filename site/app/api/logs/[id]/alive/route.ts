import {gone, ID_PATTERN} from "@/lib/config";
import {canWrite} from "@/lib/crypto";
import {getMeta, LogGone, setAlive} from "@/lib/store";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/** The most sessions one share is said to watch (more is surely a mistake). */
const MOST_SESSIONS = 100_000;
/** The most a check-in's body can be, in bytes: its summaries, mostly. */
const MOST_BYTES = 256 * 1024;

/**
 * A live share's `agent-graph watch-remote` saying it's still running:
 * POST /api/logs/{id}/alive, with the write token, like an append, and a
 * body of `{"sessions": <how many it's watching>, "host": <the computer's
 * name>, "summary": {<session id>: <its summary>}}` (host and summary from
 * 0.1.5; before, just sessions). It says so every minute or so; the home
 * page tells its owner (GET /api/watching), and /watch lists every share's
 * sessions (GET /api/watch/shares). 204; 404 for a log that isn't a live
 * share; 413 for a body over 256 KB.
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
  const text = await req.text();
  if (text.length > MOST_BYTES) {
    return new Response("Too big.", {status: 413});
  }
  let body: {sessions?: unknown; host?: unknown; summary?: unknown} | null;
  try {
    body = JSON.parse(text);
  } catch {
    body = null;
  }
  const sessions = body?.sessions;
  if (
    typeof sessions !== "number" ||
    !Number.isInteger(sessions) ||
    sessions < 0 ||
    sessions > MOST_SESSIONS
  ) {
    return new Response('Expected {"sessions": <a count>}.', {status: 400});
  }
  const host = typeof body?.host === "string" ? body.host.slice(0, 100) : "";
  const summary =
    body?.summary &&
    typeof body.summary === "object" &&
    !Array.isArray(body.summary)
      ? (body.summary as Record<string, unknown>)
      : {};
  // Only a live share has an owner to list it for.
  const meta = await getMeta(id);
  if (!meta?.owner) {
    return new Response("Not a live share.", {status: 404});
  }
  try {
    await setAlive(id, meta.owner, sessions, host, summary);
  } catch (err) {
    if (err instanceof LogGone) {
      return gone();
    }
    throw err;
  }
  return new Response(null, {status: 204});
}
