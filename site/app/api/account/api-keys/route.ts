import {currentUser, sameOrigin} from "@/lib/auth";
import {graphId, parseGraphId} from "@/lib/api/ids";
import {
  type KeyInfo,
  keysOf,
  MOST_API_KEYS,
  MOST_KEY_GRAPHS,
  newKey,
} from "@/lib/api/keys";
import {getMeta} from "@/lib/store";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/** A key as the account page shows it: its graphs by their API ids. */
const shown = (info: KeyInfo) => ({
  ...info,
  graphs: info.graphs?.map(graphId) ?? null,
});

/**
 * The logged-in account's keys to the graph API (lib/api/keys.ts): GET
 * /api/account/api-keys replies { keys: [{ id, name, kind, graphs, shown,
 * version, createdAt, usedAt }] }, newest first. 401 if not logged in.
 */
export async function GET(): Promise<Response> {
  const user = await currentUser();
  if (!user) {
    return new Response("Log in first.", {status: 401});
  }
  const keys = (await keysOf(user.uid)).map(shown);
  return Response.json({keys}, {headers: {"Cache-Control": "no-store"}});
}

/**
 * Makes a key, from the account page: POST /api/account/api-keys { name,
 * kind: "secret" | "restricted", graphs? }, where a restricted key's
 * `graphs` are the live shares it can read (their ids, `gph_…`), each the
 * account's own. Replies 201 { key, ...info }: the key, shown this once
 * (only its hash is kept). 400 for a name that's empty or too long, or
 * graphs that aren't the account's; 409 if it has as many as it can.
 */
export async function POST(req: Request): Promise<Response> {
  if (!sameOrigin(req)) {
    return new Response("Forbidden.", {status: 403});
  }
  const user = await currentUser();
  if (!user) {
    return new Response("Log in first.", {status: 401});
  }
  let body: {name?: unknown; kind?: unknown; graphs?: unknown};
  try {
    body = await req.json();
  } catch {
    return new Response("Expected JSON: { name, kind, graphs? }.", {
      status: 400,
    });
  }
  const name = typeof body.name === "string" ? body.name.trim() : "";
  if (!name || name.length > 100) {
    return new Response(
      "Give it a name (up to 100 characters): what will use it, say.",
      {status: 400},
    );
  }
  const kind = body.kind ?? "secret";
  if (kind !== "secret" && kind !== "restricted") {
    return new Response('kind is "secret" or "restricted".', {status: 400});
  }
  let graphs: Array<string> | null = null;
  if (kind === "restricted") {
    const asked = Array.isArray(body.graphs) ? body.graphs : [];
    if (!asked.length || asked.length > MOST_KEY_GRAPHS) {
      return new Response(
        `Choose the graphs a restricted key can read (1 to ${MOST_KEY_GRAPHS}).`,
        {status: 400},
      );
    }
    graphs = [];
    for (const g of asked) {
      const log = typeof g === "string" ? parseGraphId(g) : null;
      const meta = log ? await getMeta(log) : null;
      if (!log || meta?.owner !== user.uid) {
        return new Response(`${String(g)} isn't one of your live shares.`, {
          status: 400,
        });
      }
      if (!graphs.includes(log)) {
        graphs.push(log);
      }
    }
  } else if (body.graphs !== undefined) {
    return new Response("Only a restricted key is made for some graphs.", {
      status: 400,
    });
  }
  const made = await newKey(user.uid, {name, kind, graphs});
  if (!made) {
    return new Response(
      `You have ${MOST_API_KEYS} API keys already: revoke one you don't use first.`,
      {status: 409},
    );
  }
  return Response.json(
    {key: made.key, ...shown(made.info)},
    {status: 201, headers: {"Cache-Control": "no-store"}},
  );
}
