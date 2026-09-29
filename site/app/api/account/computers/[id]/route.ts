import {removeComputer} from "@/lib/accounts";
import {currentUser, sameOrigin} from "@/lib/auth";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * Logs one of the account's computers out of the CLI: DELETE
 * /api/account/computers/{id}, from the account page, with the session
 * cookie. 204, or 404 if it isn't one of theirs.
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
  return (await removeComputer(user.uid, id))
    ? new Response(null, {status: 204})
    : new Response("Not one of your computers.", {status: 404});
}
