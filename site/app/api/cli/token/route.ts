import {newCliToken, recordLogin, redeemCliCode} from "@/lib/accounts";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * The CLI's token: POST /api/cli/token { code, verifier, host }, from
 * `agent-graph watch-remote`, with the code the browser brought it and the
 * secret its challenge was made from (lib/accounts.ts). Replies
 * { token, email }. The code is used up either way.
 */
export async function POST(req: Request): Promise<Response> {
  let body: {code?: unknown; verifier?: unknown; host?: unknown};
  try {
    body = await req.json();
  } catch {
    return new Response("Expected JSON: { code, verifier, host }.", {
      status: 400,
    });
  }
  const {code, verifier, host} = body;
  if (
    typeof code !== "string" ||
    code.length > 100 ||
    typeof verifier !== "string" ||
    verifier.length > 200
  ) {
    return new Response("Expected JSON: { code, verifier, host }.", {
      status: 400,
    });
  }
  const account = await redeemCliCode(code, verifier);
  if (!account) {
    return new Response(
      "That login has expired, or was already used. Log in again.",
      {status: 401},
    );
  }
  const token = await newCliToken(
    account,
    typeof host === "string" ? host : "",
  );
  await recordLogin(account);
  return Response.json(
    {token, email: account.email},
    {headers: {"Cache-Control": "no-store"}},
  );
}
