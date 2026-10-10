// The public API showcase at /showcase: static pages built from
// ../examples/api-showcase (scripts/build-showcase.mjs), reading a demo
// account's graphs through this proxy, which holds that account's key
// (SHOWCASE_API_KEY, best a restricted key for the demo's shares). The
// browser never sees it. Only GET, and only /graphs; and since the demo's
// graphs don't change, what comes back can be cached by anyone.

/** The demo account's key, if the site has one. */
export function showcaseKey(): string | null {
  const key = process.env.SHOWCASE_API_KEY?.trim();
  return key ? key : null;
}

/** Calls the API in-process: `/api/v1/<path>` (lib/api/routes.ts). */
export type Dispatch = (req: Request, path: Array<string>) => Promise<Response>;

/** The headers passed back to the page, as the API sent them. */
const PASSED = [
  "content-type",
  "etag",
  "request-id",
  "ratelimit-limit",
  "ratelimit-remaining",
  "ratelimit-reset",
  "agent-graph-version",
  "retry-after",
];

function failure(status: number, type: string, code: string, message: string) {
  return Response.json(
    {error: {type, code, message, param: null, doc_url: ""}},
    {status, headers: {"Cache-Control": "no-store"}},
  );
}

/** Answers the showcase's GET /showcase/api/ag/<path>. */
export async function showcaseProxy(
  req: Request,
  path: Array<string>,
  {key, dispatch}: {key: string | null; dispatch: Dispatch},
): Promise<Response> {
  if (!key) {
    return failure(
      503,
      "api_error",
      "showcase_not_configured",
      "The showcase isn't set up: the site has no SHOWCASE_API_KEY.",
    );
  }
  if (path[0] !== "graphs") {
    return failure(
      404,
      "invalid_request_error",
      "resource_missing",
      "The showcase only reads /graphs.",
    );
  }
  const url = new URL(req.url);
  const target = new URL(
    `/api/v1/${path.map(encodeURIComponent).join("/")}${url.search}`,
    url,
  );
  const headers = new Headers({Authorization: `Bearer ${key}`});
  const ifNoneMatch = req.headers.get("if-none-match");
  if (ifNoneMatch) {
    headers.set("If-None-Match", ifNoneMatch);
  }
  const started = performance.now();
  const res = await dispatch(new Request(target, {headers}), path);
  const ms = Math.round(performance.now() - started);

  const out = new Headers({"X-Upstream-Ms": String(ms)});
  for (const name of PASSED) {
    const value = res.headers.get(name);
    if (value) {
      out.set(name, value);
    }
  }
  // Anyone may cache it: it's the same for everyone. A fixed point in a
  // graph's history (`as_of`) never changes; the rest only when the demo's
  // data does, which it doesn't, so a minute's staleness costs nothing.
  const upstream = res.headers.get("cache-control") ?? "";
  out.set(
    "Cache-Control",
    res.status >= 400
      ? "no-store"
      : upstream.includes("immutable")
        ? "public, max-age=31536000, immutable"
        : "public, max-age=0, s-maxage=60, stale-while-revalidate=3600",
  );
  const body = res.status === 304 ? null : await res.text();
  return new Response(body, {status: res.status, headers: out});
}
