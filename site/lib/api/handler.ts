// What every API request goes through (site/openapi.json): its id, its key
// (as a bearer token, or HTTP Basic's username), the version it's answered
// in, the key's rate limit, and the reply: JSON with an ETag (an unchanged
// one, sent back as If-None-Match, is a 304 that costs nothing), or an
// error in the API's one shape.

import {createHash, randomBytes} from "node:crypto";

import {
  ApiError,
  API_VERSION,
  API_VERSIONS,
  apiFailed,
  apiKeyInvalid,
  apiKeyMissing,
  rateLimited,
  versionInvalid,
} from "./errors";
import {type Key, shownKey} from "./keys";
import {siteUrl} from "../config";
import type {RateLimiter} from "./ratelimit";

/** A request, once its key's been checked. */
export type Ctx = {
  req: Request;
  url: URL;
  key: Key;
  /** The site's own address, for URLs in replies. */
  origin: string;
  version: string;
};

/** What an endpoint answers with. */
export type Reply = {
  body: unknown;
  /** Whether it's for a fixed point in a graph's history, so never changes. */
  immutable?: boolean;
};

export type HandlerDeps = {
  keyOf: (secret: string) => Promise<Key | null>;
  limiter: RateLimiter;
};

/** The key a request sends: `Authorization: Bearer <key>`, or Basic auth's username. */
export function credentialOf(req: Request): string | null {
  const header = req.headers.get("authorization")?.trim() ?? "";
  const [scheme, value = ""] = header.split(/\s+/, 2);
  if (/^bearer$/i.test(scheme)) {
    return value || null;
  }
  if (/^basic$/i.test(scheme)) {
    const decoded = Buffer.from(value, "base64").toString("utf8");
    const user = decoded.split(":")[0];
    return user || null;
  }
  return null;
}

/** Answers `req` with `run`, once it's let through. */
export async function serve(
  req: Request,
  run: (ctx: Ctx) => Promise<Reply>,
  deps: HandlerDeps,
): Promise<Response> {
  const requestId = `req_${randomBytes(12).toString("base64url")}`;
  const url = new URL(req.url);
  // The public address, not the one the server was reached at behind its host.
  const origin = siteUrl(req);
  let version = API_VERSION;
  const headers: Record<string, string> = {
    "Request-Id": requestId,
    "Cache-Control": "private, no-cache",
  };
  let key: Key | null = null;
  try {
    const secret = credentialOf(req);
    if (!secret) {
      throw apiKeyMissing();
    }
    key = await deps.keyOf(secret);
    if (!key) {
      throw apiKeyInvalid(
        secret.startsWith("ag_") && secret.length > 15
          ? shownKey(secret)
          : "it isn't one",
      );
    }
    version = key.version;
    const asked = req.headers.get("agent-graph-version");
    if (asked !== null) {
      if (!API_VERSIONS.includes(asked)) {
        throw versionInvalid(asked);
      }
      version = asked;
    }
    const taken = deps.limiter.take(key.id);
    headers["RateLimit-Limit"] = String(taken.limit);
    headers["RateLimit-Remaining"] = String(taken.remaining);
    headers["RateLimit-Reset"] = String(taken.reset);
    if (!taken.ok) {
      throw rateLimited(taken.retryAfter);
    }
    const reply = await run({req, url, key, origin, version});
    const json = JSON.stringify(reply.body);
    const etag = `W/"${createHash("sha256").update(json).digest("base64url").slice(0, 27)}"`;
    headers.ETag = etag;
    headers["Agent-Graph-Version"] = version;
    if (reply.immutable) {
      headers["Cache-Control"] = "private, max-age=31536000, immutable";
    }
    const match = req.headers.get("if-none-match");
    if (match && match.split(",").some(t => t.trim() === etag)) {
      deps.limiter.giveBack(key.id);
      delete headers["Agent-Graph-Version"];
      return new Response(null, {status: 304, headers});
    }
    return new Response(json, {
      status: 200,
      headers: {...headers, "Content-Type": "application/json; charset=utf-8"},
    });
  } catch (err) {
    const error = err instanceof ApiError ? err : null;
    if (!error) {
      console.error(`${requestId}: ${req.method} ${url.pathname}:`, err);
    }
    const e = error ?? apiFailed();
    return new Response(JSON.stringify(e.body(`${origin}/docs/reference`)), {
      status: e.status,
      headers: {
        ...headers,
        ...e.headers,
        "Agent-Graph-Version": version,
        "Cache-Control": "no-store",
        "Content-Type": "application/json; charset=utf-8",
      },
    });
  }
}
