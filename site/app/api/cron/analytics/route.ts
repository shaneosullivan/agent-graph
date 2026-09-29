import {deleteOldDays} from "@/lib/analytics";
import {cronAllowed} from "@/lib/cleanup";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";
export const maxDuration = 60;

/**
 * Deletes each day's counts once it's a year old, keeping its month's
 * (lib/analytics.ts): GET /api/cron/analytics, run daily by Vercel Cron
 * (vercel.json), with `Authorization: Bearer <CRON_SECRET>`. Replies
 * `{deleted}`: how many documents went.
 */
export async function GET(req: Request): Promise<Response> {
  if (!cronAllowed(req)) {
    return new Response("Not allowed.", {status: 401});
  }
  return Response.json({deleted: await deleteOldDays()});
}
