import {accountOfRequest} from "@/lib/accounts";
import {currentUser, sameOrigin} from "@/lib/auth";
import troubleshooting from "@/lib/troubleshooting.json";
import {
  type Action,
  type Breakage,
  breakagesOf,
  changeBreakage,
  MOST_REPORT_BYTES,
  parseReport,
  recordReport,
} from "@/lib/cloud-diagnostics";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * A coding agent's cloud says what's wrong with it (lib/cloud-diagnostics.ts):
 *
 *   POST  (`Authorization: Bearer <API token>`, from `agent-graph` in the
 *         cloud) a report: { cloud, findings, … }. 200 { key }, the
 *         breakage's (null for a report with no problems, which clears the
 *         cloud's).
 *   GET   the breakages to show, as { reports: [...] }; `?muted=1` for the
 *         muted ones. From the site's pages (the session cookie) or the
 *         local viewer (a CLI token).
 *   PATCH { key, action: "dismiss" | "mute" | "unmute" }. 204, or 404 if
 *         it isn't one of the account's.
 */

/** The account asking: a CLI token's, or the browser's (from this site's pages, for a change). */
async function uidOf(req: Request, changing: boolean): Promise<string | null> {
  const account = await accountOfRequest(req);
  if (account) {
    return account.uid;
  }
  if (changing && !sameOrigin(req)) {
    return null;
  }
  return (await currentUser())?.uid ?? null;
}

export async function POST(req: Request): Promise<Response> {
  const account = await accountOfRequest(req);
  if (!account) {
    return new Response("That login isn't one this site knows.", {
      status: 401,
    });
  }
  const text = await req.text();
  if (text.length > MOST_REPORT_BYTES) {
    return new Response("That report is too big.", {status: 413});
  }
  let body: unknown;
  try {
    body = JSON.parse(text);
  } catch {
    return new Response("That isn't JSON.", {status: 400});
  }
  const report = parseReport(body);
  if (!report) {
    return new Response("That isn't a report from a cloud.", {status: 400});
  }
  const key = await recordReport(account.uid, report);
  return Response.json({key});
}

export async function GET(req: Request): Promise<Response> {
  const uid = await uidOf(req, false);
  if (!uid) {
    return new Response("Log in first.", {status: 401});
  }
  const url = new URL(req.url);
  const muted = url.searchParams.get("muted") === "1";
  const reports = (await breakagesOf(uid, muted)).map(b =>
    withFixes(b, url.origin),
  );
  return Response.json({reports}, {headers: {"Cache-Control": "no-store"}});
}

type Issue = {
  id: string;
  title: string;
  fix: string;
  clouds?: Record<string, string>;
};
const ISSUES = new Map(
  (troubleshooting as unknown as {issues: Array<Issue>}).issues.map(i => [
    i.id,
    i,
  ]),
);

/**
 * Breakage `b` with what to do about each problem its report names, in its
 * cloud (docs/troubleshooting.json, which /troubleshooting shows): for the
 * viewer, whose dialog shows it.
 */
function withFixes(b: Breakage, origin: string) {
  const ids = new Set(
    b.report.findings.map(f => f.issue).filter((i): i is string => !!i),
  );
  const fixes = [...ids].flatMap(id => {
    const issue = ISSUES.get(id);
    return issue
      ? [
          {
            id,
            title: issue.title,
            fix: issue.clouds?.[b.cloud] ?? issue.fix,
            link: `${origin}/troubleshooting#${id}`,
          },
        ]
      : [];
  });
  return {...b, fixes};
}

const ACTIONS: Array<Action> = ["dismiss", "mute", "unmute"];

export async function PATCH(req: Request): Promise<Response> {
  const uid = await uidOf(req, true);
  if (!uid) {
    return new Response("Log in first.", {status: 401});
  }
  const body = (await req.json().catch(() => null)) as {
    key?: unknown;
    action?: unknown;
  } | null;
  const key = typeof body?.key === "string" ? body.key.slice(0, 2000) : null;
  const action = ACTIONS.find(a => a === body?.action);
  if (!key || !action) {
    return new Response(
      'Send { key, action: "dismiss" | "mute" | "unmute" }.',
      {
        status: 400,
      },
    );
  }
  return (await changeBreakage(uid, key, action))
    ? new Response(null, {status: 204})
    : new Response("Not one of your clouds' reports.", {status: 404});
}
