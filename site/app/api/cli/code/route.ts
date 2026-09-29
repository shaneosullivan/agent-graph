import {newCliCode, SECRET_PATTERN} from "@/lib/accounts";
import {currentUser, sameOrigin} from "@/lib/auth";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * Connects `agent-graph watch-remote` to the account logged in in this
 * browser (lib/accounts.ts): POST /api/cli/code { port, state, challenge },
 * from the login page, with the session cookie. Replies { redirect }: where
 * to send the browser, the CLI waiting on this computer's port `port`, with
 * a one-time code for it.
 */
export async function POST(req: Request): Promise<Response> {
  if (!sameOrigin(req)) {
    return new Response("Forbidden.", {status: 403});
  }
  const user = await currentUser();
  if (!user) {
    return new Response("Log in first.", {status: 401});
  }
  let body: {port?: unknown; state?: unknown; challenge?: unknown};
  try {
    body = await req.json();
  } catch {
    return new Response("Expected JSON: { port, state, challenge }.", {
      status: 400,
    });
  }
  const {port, state, challenge} = body;
  if (
    typeof port !== "number" ||
    !Number.isInteger(port) ||
    port < 1024 ||
    port > 65535 ||
    typeof state !== "string" ||
    !SECRET_PATTERN.test(state) ||
    typeof challenge !== "string" ||
    !SECRET_PATTERN.test(challenge)
  ) {
    return new Response("Expected JSON: { port, state, challenge }.", {
      status: 400,
    });
  }
  const code = await newCliCode(user, challenge);
  // 127.0.0.1, not localhost: another program could be listening on [::1].
  const redirect = `http://127.0.0.1:${port}/callback?${new URLSearchParams({code, state})}`;
  return Response.json({redirect}, {headers: {"Cache-Control": "no-store"}});
}
