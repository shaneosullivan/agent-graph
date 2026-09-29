import {accountOfRequest, standingOf} from "@/lib/accounts";
import {standingReply} from "@/lib/billing";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * Where the CLI's account stands: GET /api/cli/account, with
 * `Authorization: Bearer <CLI token>`. Replies { email, accountStatus,
 * freeUntil, canShare } (lib/billing.ts): `agent-graph watch-remote` asks,
 * while it waits for the account to subscribe. 401 if the login's ended.
 */
export async function GET(req: Request): Promise<Response> {
  const account = await accountOfRequest(req);
  if (!account) {
    return new Response(
      "That login has ended. Log in again: agent-graph watch-remote asks you to.",
      {status: 401},
    );
  }
  const standing = await standingOf(account.uid);
  return Response.json(
    {
      email: account.email,
      ...standingReply(standing),
      canShare: standing.canShare,
    },
    {headers: {"Cache-Control": "no-store"}},
  );
}
