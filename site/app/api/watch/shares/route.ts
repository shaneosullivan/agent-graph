import {accountOfRequest} from "@/lib/accounts";
import {currentUser} from "@/lib/auth";
import {sharesOf} from "@/lib/store";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/** How long after its last word a share's still taken to be running. */
const ALIVE_FOR_MS = 3 * 60 * 1000;

/**
 * The logged-in account's live shares, for /watch's list of every session
 * it's sharing: GET /api/watch/shares replies `{shares: [{id, host, at,
 * live, sessions, summary}]}`, most recent first, where `summary` is each
 * session's summary, as the viewer's list shows it (session id → summary),
 * as the share last said, and `live` whether it's running now. A CLI or API
 * token, as `Authorization: Bearer <token>`, reads its account's. 401 if not
 * logged in.
 */
export async function GET(req: Request): Promise<Response> {
  const user = (await currentUser()) ?? (await accountOfRequest(req));
  if (!user) {
    return new Response("Not logged in.", {status: 401});
  }
  const now = Date.now();
  const shares = (await sharesOf(user.uid)).map(share => ({
    ...share,
    live: now - share.at <= ALIVE_FOR_MS,
  }));
  return Response.json({shares}, {headers: {"Cache-Control": "no-store"}});
}
