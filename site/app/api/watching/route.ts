import {currentUser} from "@/lib/auth";
import {sharesOf} from "@/lib/store";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * How long after `watch-remote` last said it's running (every minute or so,
 * see /api/logs/{id}/alive) it's still taken to be: a couple of missed
 * words' grace.
 */
const ALIVE_FOR_MS = 3 * 60 * 1000;

/**
 * Whether any of the logged-in account's `agent-graph watch-remote`s (on
 * any of its computers or cloud instances) is running now, for the home
 * page: GET /api/watching replies `{watching: true, sessions}` (how many
 * sessions they're watching, together) or `{watching: false}`. 401 if not
 * logged in.
 */
export async function GET(): Promise<Response> {
  const user = await currentUser();
  if (!user) {
    return new Response("Not logged in.", {status: 401});
  }
  const running = (await sharesOf(user.uid)).filter(
    share => Date.now() - share.at <= ALIVE_FOR_MS,
  );
  const sessions = running.reduce((sum, share) => sum + share.sessions, 0);
  return Response.json(
    running.length ? {watching: true, sessions} : {watching: false},
    {headers: {"Cache-Control": "no-store"}},
  );
}
