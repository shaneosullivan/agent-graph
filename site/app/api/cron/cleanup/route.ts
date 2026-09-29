import {cronAllowed, deleteIdleLogs} from "@/lib/cleanup";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";
export const maxDuration = 60;

/** How long a run spends looking at logs, well within `maxDuration`. */
const BUDGET_MS = 45_000;

/**
 * Deletes logs that have had no event for a week (lib/cleanup.ts):
 * GET /api/cron/cleanup, run daily by Vercel Cron (vercel.json), which
 * sends `Authorization: Bearer <CRON_SECRET>`. Without CRON_SECRET set,
 * it's refused. Replies `{checked, deleted, done}`; a run that runs out of
 * time carries on next time.
 */
export async function GET(req: Request): Promise<Response> {
  if (!cronAllowed(req)) {
    return new Response("Not allowed.", {status: 401});
  }
  return Response.json(await deleteIdleLogs({budgetMs: BUDGET_MS}));
}
