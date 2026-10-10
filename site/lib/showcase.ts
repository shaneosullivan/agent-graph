// The API showcase at /showcase (app/showcase): pages that read graphs
// through the graph API from the browser, the way any integration would,
// with a key they're given when they start (app/api/showcase/key):
//
// - the demo's: the site's SHOWCASE_API_KEY, a restricted key for a demo
//   account's shares. It's handed to every browser that asks, so it's
//   public: read-only, and reading only the demo's graphs.
// - a browser's own, once it's logged in: a key made for it there and
//   then, reading every graph its account owns, for an hour
//   (lib/api/keys.ts `newBrowserKey`). It's kept only in the page's memory.

/** The demo account's key, if the site has one. */
export function showcaseKey(): string | null {
  const key = process.env.SHOWCASE_API_KEY?.trim();
  return key ? key : null;
}

export type ShowcaseSource = "demo" | "mine";

/** What POST /api/showcase/key answers: a key, or why there isn't one. */
export async function showcaseKeyReply(
  source: unknown,
  {
    demoKey,
    uid,
    mint,
  }: {
    demoKey: string | null;
    /** The account logged in in the browser that asked; null if none. */
    uid: string | null;
    mint: (uid: string) => Promise<{key: string; expires: number}>;
  },
): Promise<Response> {
  const fail = (status: number, code: string, message: string) =>
    Response.json(
      {error: {code, message}},
      {status, headers: {"Cache-Control": "no-store"}},
    );
  const ok = (body: {key: string; expires: number | null}) =>
    Response.json(body, {headers: {"Cache-Control": "no-store"}});

  if (source === "demo") {
    return demoKey
      ? ok({key: demoKey, expires: null})
      : fail(
          503,
          "showcase_not_configured",
          "The showcase isn't set up: the site has no SHOWCASE_API_KEY.",
        );
  }
  if (source === "mine") {
    return uid
      ? ok(await mint(uid))
      : fail(401, "not_logged_in", "Log in to read your own graphs here.");
  }
  return fail(400, "source_invalid", 'source is "demo" or "mine".');
}
