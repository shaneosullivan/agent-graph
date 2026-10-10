// GET /showcase/api/ag/…: the API showcase's proxy (lib/showcase.ts).
import {dispatch} from "@/lib/api/routes";
import {showcaseKey, showcaseProxy} from "@/lib/showcase";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function GET(
  req: Request,
  {params}: {params: Promise<{path: Array<string>}>},
): Promise<Response> {
  const {path} = await params;
  return showcaseProxy(req, path, {key: showcaseKey(), dispatch});
}
