import {API_URL, apiKey} from "@/lib/server";

export const dynamic = "force-dynamic";

/**
 * The browser's way to the Agent Graph API: GET /api/ag/graphs/… is sent on
 * to the API's /graphs/…, with your key (from .env) added here, on the
 * server, so the browser never sees it. Only GET, and only /graphs: this
 * app reads, nothing else. What comes back is passed on as it is, with the
 * headers worth showing (Request-Id, ETag, the rate limit) and how long the
 * API took.
 */
export async function GET(
  req: Request,
  {params}: {params: Promise<{path: Array<string>}>},
): Promise<Response> {
  const key = apiKey();
  if (!key) {
    return Response.json(
      {
        error: {
          type: "authentication_error",
          code: "api_key_missing",
          message:
            "This app has no API key yet: copy .env.example to .env, put your key in AGENT_GRAPH_KEY, and restart it.",
          param: null,
          doc_url:
            "https://agentgraph.chofter.com/docs/reference#authentication",
        },
      },
      {status: 401},
    );
  }
  const {path} = await params;
  if (path[0] !== "graphs") {
    return Response.json(
      {
        error: {
          type: "invalid_request_error",
          code: "resource_missing",
          message: "This app only reads /graphs.",
          param: null,
          doc_url: "",
        },
      },
      {status: 404},
    );
  }
  const search = new URL(req.url).search;
  const url = `${API_URL}/${path.map(encodeURIComponent).join("/")}${search}`;
  const ifNoneMatch = req.headers.get("if-none-match");
  const started = performance.now();
  let res: Response;
  try {
    res = await fetch(url, {
      headers: {
        Authorization: `Bearer ${key}`,
        ...(ifNoneMatch ? {"If-None-Match": ifNoneMatch} : {}),
      },
      cache: "no-store",
    });
  } catch (err) {
    return Response.json(
      {
        error: {
          type: "api_error",
          code: "unreachable",
          message: `Couldn't reach ${API_URL}: ${err instanceof Error ? err.message : String(err)}`,
          param: null,
          doc_url: "",
        },
      },
      {status: 502},
    );
  }
  const ms = Math.round(performance.now() - started);
  const headers = new Headers({"X-Upstream-Ms": String(ms)});
  for (const name of [
    "content-type",
    "etag",
    "request-id",
    "ratelimit-limit",
    "ratelimit-remaining",
    "agent-graph-version",
    "retry-after",
  ]) {
    const value = res.headers.get(name);
    if (value) headers.set(name, value);
  }
  const body = res.status === 304 ? null : await res.text();
  return new Response(body, {status: res.status, headers});
}
