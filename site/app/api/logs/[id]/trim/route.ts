import {gone, ID_PATTERN} from "@/lib/config";
import {canWrite} from "@/lib/crypto";
import {LogGone, trimLog} from "@/lib/store";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * Deletes the start of a log: POST /api/logs/{id}/trim?before={offset}
 *
 * With the write token, like an append. A live share (`agent-graph
 * watch-remote`) keeps only its last two keyframes' worth: when it sends a
 * keyframe, it asks for the chunks before the previous one to go, so the
 * log starts at that keyframe.
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
  const before = new URL(req.url).searchParams.get("before") ?? "";
  if (!/^\d{1,15}$/.test(before)) {
    return new Response("Bad offset.", {status: 400});
  }
  try {
    await trimLog(id, Number(before));
  } catch (err) {
    if (err instanceof LogGone) {
      return gone();
    }
    throw err;
  }
  return new Response(null, {status: 204});
}
