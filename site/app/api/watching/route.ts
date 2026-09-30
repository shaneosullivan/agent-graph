import {watchLog} from "@/lib/accounts";
import {currentUser} from "@/lib/auth";
import {getAlive, getMeta} from "@/lib/store";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * How long after `watch-remote` last said it's running (every minute or so,
 * see /api/logs/{id}/alive) it's still taken to be: a couple of missed
 * words' grace.
 */
const ALIVE_FOR_MS = 3 * 60 * 1000;

/**
 * Whether the logged-in account's `agent-graph watch-remote` is running
 * now, for the home page: GET /api/watching replies `{watching: true,
 * sessions}` (how many sessions it's watching) or `{watching: false}`. 401
 * if not logged in.
 */
export async function GET(): Promise<Response> {
  const user = await currentUser();
  if (!user) {
    return new Response("Not logged in.", {status: 401});
  }
  const id = await watchLog(user.uid);
  const meta = id ? await getMeta(id) : null;
  const alive = id && meta?.owner === user.uid ? await getAlive(id) : null;
  const watching = alive !== null && Date.now() - alive.at <= ALIVE_FOR_MS;
  return Response.json(
    watching ? {watching, sessions: alive.sessions} : {watching},
    {headers: {"Cache-Control": "no-store"}},
  );
}
