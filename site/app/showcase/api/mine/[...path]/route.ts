// GET /showcase/api/mine/…: the API showcase's proxy for a browser that's
// logged in, reading its own account's graphs (lib/showcase.ts).
import {dispatch} from "@/lib/api/routes";
import {currentUser} from "@/lib/auth";
import {mineProxy} from "@/lib/showcase";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function GET(
  req: Request,
  {params}: {params: Promise<{path: Array<string>}>},
): Promise<Response> {
  const {path} = await params;
  const user = await currentUser();
  return mineProxy(req, path, {uid: user?.uid ?? null, dispatch});
}
