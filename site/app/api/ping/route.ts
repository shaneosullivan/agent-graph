/**
 * GET /api/ping?n=<number>: replies with the same number, as plain text,
 * and status 200; 400 if `n` isn't a whole number. No login. The CLI
 * pings it to check it can reach the site (`agent-graph watch-remote
 * --check`, and installing for a coding agent's cloud): getting its own,
 * random number back means it reached this site, not a proxy's error page.
 */
export const dynamic = "force-dynamic";

export function GET(req: Request): Response {
  const n = new URL(req.url).searchParams.get("n")?.trim() ?? "";
  const headers = {"content-type": "text/plain", "cache-control": "no-store"};
  if (!/^-?\d{1,15}$/.test(n)) {
    return new Response("Give a whole number: /api/ping?n=42\n", {
      status: 400,
      headers,
    });
  }
  return new Response(n, {status: 200, headers});
}
