import {currentUser, sameOrigin} from "@/lib/auth";
import {revokeKey} from "@/lib/api/keys";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * Revokes one of the account's keys to the graph API: DELETE
 * /api/account/api-keys/{id}, from the account page, with the session
 * cookie. It stops working at once. 204, or 404 if it isn't one of theirs.
 */
export async function DELETE(
  req: Request,
  {params}: {params: Promise<{id: string}>},
): Promise<Response> {
  if (!sameOrigin(req)) {
    return new Response("Forbidden.", {status: 403});
  }
  const user = await currentUser();
  if (!user) {
    return new Response("Log in first.", {status: 401});
  }
  const {id} = await params;
  return (await revokeKey(user.uid, id))
    ? new Response(null, {status: 204})
    : new Response("Not one of your keys.", {status: 404});
}
