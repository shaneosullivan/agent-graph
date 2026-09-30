import {MOST_API_TOKENS, newApiToken} from "@/lib/accounts";
import {currentUser, sameOrigin} from "@/lib/auth";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * Makes an API token for the logged-in account: POST /api/account/tokens
 * { name }, from the account page. Replies { token, id }: the token, which
 * is shown this once (only its hash is kept), and its id, for revoking it
 * (DELETE /api/account/computers/{id}, as a computer's login is). 400 for a
 * name that's empty or too long; 409 if the account has as many as it can.
 */
export async function POST(req: Request): Promise<Response> {
  if (!sameOrigin(req)) {
    return new Response("Forbidden.", {status: 403});
  }
  const user = await currentUser();
  if (!user) {
    return new Response("Log in first.", {status: 401});
  }
  let name: unknown;
  try {
    ({name} = await req.json());
  } catch {
    return new Response("Expected JSON: { name }.", {status: 400});
  }
  const trimmed = typeof name === "string" ? name.trim() : "";
  if (!trimmed || trimmed.length > 100) {
    return new Response(
      "Give it a name (up to 100 characters): where it's used, say.",
      {status: 400},
    );
  }
  const made = await newApiToken({uid: user.uid, email: user.email}, trimmed);
  if (!made) {
    return new Response(
      `You have ${MOST_API_TOKENS} API tokens already: revoke one you don't use first.`,
      {status: 409},
    );
  }
  return Response.json(made, {
    status: 201,
    headers: {"Cache-Control": "no-store"},
  });
}
