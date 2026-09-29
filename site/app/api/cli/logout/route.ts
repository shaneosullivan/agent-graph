import {accountOfToken, deleteCliToken} from "@/lib/accounts";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * Logs a computer's CLI out: POST /api/cli/logout, with
 * `Authorization: Bearer <its token>` (`agent-graph watch-remote --logout`).
 * The token stops working. 204, whether or not it was one.
 */
export async function POST(req: Request): Promise<Response> {
  const header = req.headers.get("authorization") ?? "";
  const token = header.startsWith("Bearer ") ? header.slice(7).trim() : "";
  if (token && (await accountOfToken(token))) {
    await deleteCliToken(token);
  }
  return new Response(null, {status: 204});
}
