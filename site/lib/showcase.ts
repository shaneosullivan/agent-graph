// The API showcase at /showcase (app/showcase): pages that read a demo
// account's graphs through this proxy, which holds that account's key
// (SHOWCASE_API_KEY, best a restricted key for the demo's shares). The
// browser never sees it. Only GET, and only /graphs; and since the demo's
// graphs don't change, what comes back can be cached by anyone.
//
// A browser that's logged in can read its own account's graphs instead,
// through a proxy of their own (/showcase/api/mine/…, `mineProxy`), as its
// account, by its session: what comes back is for it alone.

import {API_VERSION} from "./api/errors";
import type {Key} from "./api/keys";

/** The demo account's key, if the site has one. */
export function showcaseKey(): string | null {
  const key = process.env.SHOWCASE_API_KEY?.trim();
  return key ? key : null;
}

/** Calls the API in-process: `/api/v1/<path>` (lib/api/routes.ts). */
export type Dispatch = (
  req: Request,
  path: Array<string>,
  as?: Key,
) => Promise<Response>;

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

/** Answers the showcase's GET /showcase/api/ag/<path>, with the demo's key. */
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
  return forward(req, path, {
    dispatch,
    headers: {Authorization: `Bearer ${key}`},
    shared: true,
  });
}

/**
 * Answers GET /showcase/api/mine/<path>: the same, as the account `uid`
 * that's logged in in this browser (none: it isn't), reading every graph it
 * owns, as a secret key would. Never cached for anyone else.
 */
export async function mineProxy(
  req: Request,
  path: Array<string>,
  {uid, dispatch}: {uid: string | null; dispatch: Dispatch},
): Promise<Response> {
  if (!uid) {
    return failure(
      401,
      "authentication_error",
      "not_logged_in",
      "Log in to read your own graphs here.",
    );
  }
  const as: Key = {
    id: `session:${uid}`,
    uid,
    kind: "secret",
    graphs: null,
    version: API_VERSION,
  };
  return forward(req, path, {
    dispatch,
    // Answered as `as`, whatever it says (lib/api/routes.ts `dispatch`).
    headers: {Authorization: "Bearer session"},
    as,
    shared: false,
  });
}

async function forward(
  req: Request,
  path: Array<string>,
  opts: {
    dispatch: Dispatch;
    headers: Record<string, string>;
    as?: Key;
    /** Whether what comes back is the same for everyone. */
    shared: boolean;
  },
): Promise<Response> {
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
  const headers = new Headers(opts.headers);
  const ifNoneMatch = req.headers.get("if-none-match");
  if (ifNoneMatch) {
    headers.set("If-None-Match", ifNoneMatch);
  }
  const started = performance.now();
  const res = await opts.dispatch(
    new Request(target, {headers}),
    path,
    opts.as,
  );
  const ms = Math.round(performance.now() - started);

  const out = new Headers({"X-Upstream-Ms": String(ms)});
  for (const name of PASSED) {
    const value = res.headers.get(name);
    if (value) {
      out.set(name, value);
    }
  }
  // The demo's replies are the same for everyone, so anyone may cache them.
  // A fixed point in a graph's history (`as_of`) never changes; the rest only
  // when the demo's data does, which it doesn't, so a minute's staleness
  // costs nothing. An account's own are its alone.
  const upstream = res.headers.get("cache-control") ?? "";
  out.set(
    "Cache-Control",
    res.status >= 400
      ? "no-store"
      : !opts.shared
        ? "private, no-cache"
        : upstream.includes("immutable")
          ? "public, max-age=31536000, immutable"
          : "public, max-age=0, s-maxage=60, stale-while-revalidate=3600",
  );
  const body = res.status === 304 ? null : await res.text();
  return new Response(body, {status: res.status, headers: out});
}
