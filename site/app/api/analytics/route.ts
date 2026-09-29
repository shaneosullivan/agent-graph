import {countEach} from "@/lib/analytics";
import {analyticsDisabled, countsOf} from "@/lib/analytics-core";
import {sameOrigin} from "@/lib/auth";
import {bodyText} from "@/lib/config";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * A browser's report of what it did: POST /api/analytics, from the site's
 * own pages (lib/analytics-client.ts), with a small JSON body: a page
 * shown, or a download clicked (what's counted for each:
 * lib/analytics-core.ts, `countsOf`). 204 whatever it was, and whether or
 * not it was counted (with ANALYTICS_DISABLED=true, nothing is); 403 from
 * elsewhere.
 */
export async function POST(req: Request): Promise<Response> {
  if (!sameOrigin(req)) {
    return new Response("Forbidden.", {status: 403});
  }
  if (analyticsDisabled()) {
    return new Response(null, {status: 204});
  }
  const text = await bodyText(req, 1024);
  let report: unknown = null;
  try {
    report = text ? JSON.parse(text) : null;
  } catch {
    // Nothing to count.
  }
  const counts = countsOf(report);
  if (counts) {
    await countEach(counts.day, counts.month).catch(err =>
      console.error("Counting a report:", err),
    );
  }
  return new Response(null, {status: 204});
}
