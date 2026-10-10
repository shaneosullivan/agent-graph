import {newBrowserKey} from "@/lib/api/keys";
import {currentUser, sameOrigin} from "@/lib/auth";
import {showcaseKey, showcaseKeyReply} from "@/lib/showcase";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * A key for the API showcase to read graphs with, from the browser
 * (lib/showcase.ts): POST /api/showcase/key { source: "demo" | "mine" }
 * replies { key, expires }: the demo's key (expires null), or for a
 * browser that's logged in, a key of its account's own for an hour.
 * 401 for "mine" if it isn't logged in; 503 for "demo" if the site has no
 * demo key.
 */
export async function POST(req: Request): Promise<Response> {
  if (!sameOrigin(req)) {
    return new Response("Forbidden.", {status: 403});
  }
  let body: {source?: unknown};
  try {
    body = await req.json();
  } catch {
    body = {};
  }
  const user = body.source === "mine" ? await currentUser() : null;
  return showcaseKeyReply(body.source, {
    demoKey: showcaseKey(),
    uid: user?.uid ?? null,
    mint: uid => newBrowserKey(uid),
  });
}
