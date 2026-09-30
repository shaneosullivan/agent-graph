import {gone, ID_PATTERN} from "@/lib/config";
import {canWrite} from "@/lib/crypto";
import {LogGone, setAlive} from "@/lib/store";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/** The most sessions one share is said to watch (more is surely a mistake). */
const MOST_SESSIONS = 100_000;

/**
 * A live share's `agent-graph watch-remote` saying it's still running:
 * POST /api/logs/{id}/alive, with the write token, like an append, and a
 * body of `{"sessions": <how many it's watching>}`. It says so every minute
 * or so; the home page tells its owner (GET /api/watching). 204.
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
  const body = (await req.json().catch(() => null)) as {
    sessions?: unknown;
  } | null;
  const sessions = body?.sessions;
  if (
    typeof sessions !== "number" ||
    !Number.isInteger(sessions) ||
    sessions < 0 ||
    sessions > MOST_SESSIONS
  ) {
    return new Response('Expected {"sessions": <a count>}.', {status: 400});
  }
  try {
    await setAlive(id, sessions);
  } catch (err) {
    if (err instanceof LogGone) {
      return gone();
    }
    throw err;
  }
  return new Response(null, {status: 204});
}
