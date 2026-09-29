import {accountOfRequest, setWatchLog, standingOf} from "@/lib/accounts";
import {count} from "@/lib/analytics";
import {mustSubscribe, standingReply} from "@/lib/billing";
import {ID_PATTERN, siteUrl} from "@/lib/config";
import {safeEqual, writeToken} from "@/lib/crypto";
import {getMeta, setUntil} from "@/lib/store";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * Makes an account's live share the one /watch shows them again: POST
 * /api/logs/{id}/watch, when `agent-graph watch-remote` carries on with it,
 * with `Authorization: Bearer <CLI token>` and the log's write token in
 * `X-Agent-Graph-Write-Token`. Replies { url, accountStatus, freeUntil }:
 * /watch, and where the account stands (lib/billing.ts). Appends to it are
 * allowed again till the account's time is next up: after a renewal, say.
 * 402 if it has to subscribe first. 404 if it isn't a share of theirs, or
 * is gone.
 */
export async function POST(
  req: Request,
  {params}: {params: Promise<{id: string}>},
): Promise<Response> {
  const {id} = await params;
  if (!ID_PATTERN.test(id)) {
    return new Response("Unknown log.", {status: 404});
  }
  const account = await accountOfRequest(req);
  if (!account) {
    return new Response(
      "That login has ended. Log in again: agent-graph watch-remote asks you to.",
      {
        status: 401,
      },
    );
  }
  if (
    !safeEqual(req.headers.get("x-agent-graph-write-token"), writeToken(id))
  ) {
    return new Response("Bad or missing write token.", {status: 401});
  }
  const meta = await getMeta(id);
  if (!meta || meta.owner !== account.uid) {
    return new Response("Not a share of yours.", {status: 404});
  }
  const standing = await standingOf(account.uid);
  if (!standing.canShare) {
    return mustSubscribe(siteUrl(req));
  }
  await setUntil(id, standing.until);
  await setWatchLog(account.uid, id);
  // A live share, carried on with (lib/analytics-core.ts).
  await count("watch");
  return Response.json(
    {url: `${siteUrl(req)}/watch`, ...standingReply(standing)},
    {headers: {"Cache-Control": "no-store"}},
  );
}
