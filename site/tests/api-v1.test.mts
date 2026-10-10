// Unit tests for the graph API's endpoints (lib/api/v1.ts), each request
// answered through lib/api/handler.ts as the site answers it, from a fake
// store holding the example logs:  npm run test:unit
//
// tests/api-v1-site.test.mjs tests the same against the running site,
// with Firestore and the account pages.

import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {register} from "node:module";
import {test} from "node:test";

register("../scripts/resolve-ts.mjs", import.meta.url);
const {serve} = await import("../lib/api/handler.ts");
const v1 = await import("../lib/api/v1.ts");
const {Graphs} = await import("../lib/api/source.ts");
const {RateLimiter} = await import("../lib/api/ratelimit.ts");
const {nodeId, parseNodeId, eventId} = await import("../lib/api/ids.ts");
const {chunkKey} = await import("../lib/store.ts");
const {examples, trimmed, line, chunks} = await import("./api-v1-fixtures.mts");
type Key = import("../lib/api/keys.ts").Key;
type Share = import("../lib/store.ts").Share;

const spec = JSON.parse(
  readFileSync(new URL("../openapi.json", import.meta.url), "utf8"),
);
const required = (schema: string) =>
  [...spec.components.schemas[schema].required].sort();

// ---------- a fake store ----------

type Log = {
  owner?: string;
  source: "watch" | "paste" | "upload";
  createdAt: number;
  /** Chunks, by offset. */
  chunks: Map<number, string>;
  alive: boolean;
};

const store = new Map<string, Log>();
let reads = 0;

/** How many chunks a read returns (lib/store.ts reads up to 200; fewer, to page). */
const PER_READ = 3;

function putLog(
  id: string,
  text: string,
  {
    owner = "alice",
    source = "watch",
    alive = true,
    chunkSize = 4000,
  }: {
    /** null: a pasted log, no account's. */
    owner?: string | null;
    source?: Log["source"];
    alive?: boolean;
    chunkSize?: number;
  } = {},
): void {
  const log: Log = {
    owner: owner ?? undefined,
    source,
    createdAt: Date.UTC(2026, 8, 1),
    chunks: new Map(),
    alive,
  };
  store.set(id, log);
  appendText(id, text, chunkSize);
}

function appendText(id: string, text: string, chunkSize = 4000): void {
  const log = store.get(id)!;
  let offset = 0;
  for (const [at, c] of log.chunks) {
    offset = Math.max(offset, at + Buffer.byteLength(c));
  }
  for (const c of chunks(text, chunkSize)) {
    log.chunks.set(offset, c);
    offset += Buffer.byteLength(c);
  }
}

/** Deletes the chunks before `offset` (a live share trims its start). */
function trimBefore(id: string, offset: number): void {
  const log = store.get(id)!;
  for (const at of [...log.chunks.keys()]) {
    if (at < offset) {
      log.chunks.delete(at);
    }
  }
}

/** lib/store.ts readChunks, over the fake store: stops at a gap, as it does. */
async function readChunks(id: string, after: string) {
  reads += 1;
  const log = store.get(id);
  const offsets = [...(log?.chunks.keys() ?? [])]
    .sort((a, b) => a - b)
    .filter(o => !after || o > Number(after));
  const texts: Array<string> = [];
  let first: string | null = null;
  let last: string | null = null;
  let next: number | null = null;
  for (const o of offsets) {
    if (texts.length >= PER_READ) {
      return {text: texts.join(""), first, last, more: true};
    }
    if (next !== null && o !== next) {
      return {text: texts.join(""), first, last, more: true};
    }
    const text = log!.chunks.get(o)!;
    texts.push(text);
    first ??= chunkKey(o);
    last = chunkKey(o);
    next = o + Buffer.byteLength(text);
  }
  return {text: texts.join(""), first, last, more: false};
}

async function getMeta(id: string) {
  const log = store.get(id);
  if (!log) return null;
  return {
    source: log.source,
    ...(log.owner ? {owner: log.owner} : {}),
    createdAt: {toMillis: () => log.createdAt},
  } as unknown as Awaited<ReturnType<typeof import("../lib/store.ts").getMeta>>;
}

async function sharesOf(uid: string): Promise<Array<Share>> {
  return [...store]
    .filter(([, log]) => log.owner === uid && log.alive)
    .map(([id]) => ({id, host: "mbp", at: 0, sessions: 1, summary: {}}));
}

let clock = Date.UTC(2026, 9, 9, 12);
const keys = new Map<string, Key>();
const key = (secret: string, k: Partial<Key> = {}) => {
  keys.set(secret, {
    id: `id-${secret}`,
    uid: "alice",
    kind: "secret",
    graphs: null,
    version: "2026-10-09",
    ...k,
  });
  return secret;
};
const ALICE = key("ag_sk_live_aliceAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
const BOB = key("ag_sk_live_bobBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB", {uid: "bob"});
const ONLY_A = key("ag_rk_live_onlyAAAAAAAAAAAAAAAAAAAAAAAAAAAA", {
  kind: "restricted",
  graphs: ["ExampleLog01"],
});

let graphs = new Graphs(readChunks, () => clock);
let limiter = new RateLimiter(25, 100, () => clock);
const deps = () => ({sharesOf, getMeta, graphs, now: () => clock});

// ---------- requests ----------

type Got = {
  status: number;
  headers: Headers;
  body: Record<string, any>;
};

function route(path: string) {
  const d = deps();
  const parts = path.split("/").filter(Boolean).slice(2); // after /api/v1
  const [graphsSeg, g, sub, third] = parts;
  if (graphsSeg !== "graphs") throw new Error(path);
  if (!g) return ctx => v1.listGraphs(ctx, d);
  if (!sub) return ctx => v1.retrieveGraph(ctx, d, g);
  if (sub === "nodes" && !third) return ctx => v1.listNodes(ctx, d, g);
  if (sub === "nodes" && third === "search")
    return ctx => v1.searchNodes(ctx, d, g);
  if (sub === "nodes") return ctx => v1.retrieveNode(ctx, d, g, third);
  if (sub === "events" && !third) return ctx => v1.listEvents(ctx, d, g);
  return ctx => v1.retrieveEvent(ctx, d, g, third);
}

async function get(
  path: string,
  {
    as = ALICE,
    headers = {},
    query,
  }: {
    as?: string | null;
    headers?: Record<string, string>;
    query?: Record<string, string | Array<string>>;
  } = {},
): Promise<Got> {
  let url = `https://site.test${path}`;
  if (query) {
    const params = new URLSearchParams();
    for (const [k, v] of Object.entries(query)) {
      for (const one of [v].flat()) params.append(k, one);
    }
    url += `${path.includes("?") ? "&" : "?"}${params}`;
  }
  const req = new Request(url, {
    headers: {...(as ? {authorization: `Bearer ${as}`} : {}), ...headers},
  });
  const res = await serve(req, route(new URL(url).pathname), {
    keyOf: async (s: string) => keys.get(s) ?? null,
    limiter,
  });
  const text = await res.text();
  return {
    status: res.status,
    headers: res.headers,
    body: text ? JSON.parse(text) : null,
  };
}

/** A request that must succeed. */
async function ok(path: string, opts?: Parameters<typeof get>[1]) {
  const res = await get(path, opts);
  assert.equal(res.status, 200, JSON.stringify(res.body));
  return res.body;
}

/** A request that must fail with `status` and `code`. */
async function fails(
  path: string,
  status: number,
  code: string,
  opts?: Parameters<typeof get>[1],
) {
  const res = await get(path, opts);
  assert.equal(res.status, status, `${path}: ${JSON.stringify(res.body)}`);
  assert.equal(res.body.error.code, code, path);
  assert.deepEqual(
    Object.keys(res.body.error)
      .filter(k => !["first_event_id", "last_event_id"].includes(k))
      .sort(),
    ["code", "doc_url", "message", "param", "type"],
  );
  assert.equal(
    res.body.error.doc_url,
    "https://site.test/docs/reference#errors",
  );
  assert.match(res.headers.get("request-id")!, /^req_/);
  return res.body.error;
}

// ---------- the graphs ----------

const EXAMPLES = examples();
const G = "gph_ExampleLog01";
const P = `/api/v1/graphs/${G}`;
putLog("ExampleLog01", EXAMPLES, {chunkSize: 1500});
// A live share, as it's stored: from a keyframe, its earlier history gone.
const CUT = trimmed(EXAMPLES, 40);
putLog("TrimmedLog01", CUT);
putLog("BobsLog00001", EXAMPLES, {owner: "bob"});
putLog("PastedLog001", EXAMPLES, {owner: null, source: "paste"});
putLog("QuietLog0001", EXAMPLES, {alive: false});

const reducer = (await import("../lib/api/reducer.ts")).LogReducer;
const all = await reducer.create();
all.append(EXAMPLES);
const EVENTS = all.events().events;
const STATE = all.state();
const FIRST = eventId(EVENTS[0].id);
const LAST = eventId(EVENTS.at(-1)!.id);
const refs = Object.keys(STATE.nodes);
const someNode = (pred: (n: (typeof STATE.nodes)[string]) => boolean) =>
  refs.find(r => pred(STATE.nodes[r]))!;

// ---------- tests ----------

test("a key is needed, as a bearer token or Basic auth's username", async () => {
  const missing = await fails("/api/v1/graphs", 401, "api_key_missing", {
    as: null,
  });
  assert.equal(missing.type, "authentication_error");
  const bad = await fails("/api/v1/graphs", 401, "api_key_invalid", {
    as: "ag_sk_live_nopeNNNNNNNNNNNNNNNNNNNNNNNNNNNN",
  });
  assert.match(
    bad.message,
    /ag_sk_live_\*\*\*\*NNNN/,
    "shown, not given back whole",
  );
  await fails("/api/v1/graphs", 401, "api_key_invalid", {as: "junk"});
  const basic = Buffer.from(`${ALICE}:`).toString("base64");
  const res = await get("/api/v1/graphs", {
    as: null,
    headers: {authorization: `Basic ${basic}`},
  });
  assert.equal(res.status, 200);
  assert.match(res.headers.get("request-id")!, /^req_[A-Za-z0-9_-]{16}$/);
  assert.equal(res.headers.get("agent-graph-version"), "2026-10-09");
  assert.equal(
    res.headers.get("content-type"),
    "application/json; charset=utf-8",
  );
});

test("versions: the key's, unless one's asked for; one that isn't is refused", async () => {
  const res = await get("/api/v1/graphs", {
    headers: {"agent-graph-version": "2026-10-09"},
  });
  assert.equal(res.headers.get("agent-graph-version"), "2026-10-09");
  const e = await fails("/api/v1/graphs", 400, "version_invalid", {
    headers: {"agent-graph-version": "2020-01-01"},
  });
  assert.equal(e.type, "invalid_request_error");
});

test("listing graphs: the account's live shares, newest first, each as the spec says", async () => {
  const list = await ok("/api/v1/graphs");
  assert.deepEqual(Object.keys(list).sort(), required("GraphList"));
  assert.equal(list.object, "list");
  assert.equal(list.url, "/api/v1/graphs");
  assert.equal(list.as_of_event_id, null);
  const ids = list.data.map((g: {id: string}) => g.id).sort();
  // Not Bob's, not a pasted log, not one that's stopped saying it's running.
  assert.deepEqual(ids, ["gph_ExampleLog01", "gph_TrimmedLog01"]);
  for (const g of list.data) {
    assert.deepEqual(Object.keys(g).sort(), required("Graph"));
    assert.deepEqual(Object.keys(g.retention).sort(), required("Retention"));
  }
  const lasts = list.data.map((g: {last_event: number}) => g.last_event);
  assert.deepEqual(
    lasts,
    [...lasts].sort((a, b) => b - a),
  );

  // A restricted key lists only its own.
  const mine = await ok("/api/v1/graphs", {as: ONLY_A});
  assert.deepEqual(
    mine.data.map((g: {id: string}) => g.id),
    [G],
  );
  // Bob sees Bob's.
  const bobs = await ok("/api/v1/graphs", {as: BOB});
  assert.deepEqual(
    bobs.data.map((g: {id: string}) => g.id),
    ["gph_BobsLog00001"],
  );

  // Paged, and filtered by when they were last active.
  const page = await ok("/api/v1/graphs", {query: {limit: "1"}});
  assert.equal(page.data.length, 1);
  assert.equal(page.has_more, true);
  const next = await ok("/api/v1/graphs", {
    query: {limit: "1", starting_after: page.data[0].id},
  });
  assert.equal(next.has_more, false);
  assert.notEqual(next.data[0].id, page.data[0].id);
  const none = await ok("/api/v1/graphs", {
    query: {"last_event[gt]": String(clock)},
  });
  assert.deepEqual(none.data, []);
  await fails("/api/v1/graphs", 400, "parameter_unknown", {
    query: {as_of: FIRST},
  });
});

test("a graph: what it holds, and how its nodes stand, now or at any event", async () => {
  const g = await ok(P);
  assert.equal(g.id, G);
  assert.equal(g.object, "graph");
  assert.equal(g.source, "watch");
  assert.equal(g.url, "https://site.test/l/ExampleLog01");
  assert.equal(g.as_of_event_id, LAST);
  assert.deepEqual(g.retention, {
    first_event_id: FIRST,
    last_event_id: LAST,
    first_event_created: EVENTS[0].ms,
    last_event_created: EVENTS.at(-1)!.ms,
    event_count: EVENTS.length,
  });
  assert.equal(g.last_event, EVENTS.at(-1)!.ms);
  assert.equal(g.counts.nodes, refs.length);

  // A share link's id works too.
  assert.equal((await ok("/api/v1/graphs/ExampleLog01")).id, G);

  // Links are to the site's public address, not the one it was reached at.
  process.env.NEXT_PUBLIC_SITE_URL = "https://agentgraph.example/";
  try {
    assert.equal(
      (await ok(P)).url,
      "https://agentgraph.example/l/ExampleLog01",
    );
    assert.equal(
      (await get(P, {query: {nope: "1"}})).body.error.doc_url,
      "https://agentgraph.example/docs/reference#errors",
    );
  } finally {
    delete process.env.NEXT_PUBLIC_SITE_URL;
  }

  // At its first event: one node.
  const first = await get(P, {query: {as_of: FIRST}});
  assert.equal(first.status, 200);
  assert.equal(first.body.as_of_event_id, FIRST);
  assert.equal(first.body.counts.nodes, 1);
  // A fixed point never changes.
  assert.equal(
    first.headers.get("cache-control"),
    "private, max-age=31536000, immutable",
  );
  assert.equal(
    (await get(P)).headers.get("cache-control"),
    "private, no-cache",
  );

  // By time: the last event at or before it.
  const mid = EVENTS[40];
  const byTime = await ok(P, {query: {as_of: String(mid.ms)}});
  const atOrBefore = EVENTS.filter(e => e.ms <= mid.ms).at(-1)!;
  assert.equal(byTime.as_of_event_id, eventId(atOrBefore.id));

  // Before what it holds: 410, saying what it holds.
  const gone = await fails(P, 410, "outside_retention_window", {
    query: {as_of: String(EVENTS[0].ms - 1)},
  });
  assert.equal(gone.first_event_id, FIRST);
  assert.equal(gone.last_event_id, LAST);
  assert.equal(gone.param, "as_of");
  await fails(P, 410, "outside_retention_window", {
    query: {as_of: "evt_00000000000000000000000000"},
  });
  await fails(P, 404, "resource_missing", {
    query: {as_of: "evt_ZZZZZZZZZZZZZZZZZZZZZZZZZZ"},
  });
});

test("a graph from a keyframe holds only its recent history", async () => {
  const g = await ok("/api/v1/graphs/gph_TrimmedLog01");
  assert.ok(g.retention.event_count < EVENTS.length);
  assert.notEqual(g.retention.first_event_id, FIRST);
  // Yet every node is there, as it stands now.
  assert.equal(g.counts.nodes, refs.length);
  await fails(
    "/api/v1/graphs/gph_TrimmedLog01",
    410,
    "outside_retention_window",
    {
      query: {as_of: FIRST},
    },
  );
});

test("only the account's own graphs, and only those a restricted key was made for", async () => {
  await fails("/api/v1/graphs/gph_BobsLog00001", 404, "resource_missing");
  await fails("/api/v1/graphs/gph_PastedLog001", 404, "resource_missing");
  await fails("/api/v1/graphs/gph_NoSuchLog001", 404, "resource_missing");
  const bad = await fails(
    "/api/v1/graphs/not-a-graph",
    404,
    "resource_missing",
  );
  assert.equal(bad.param, "graph_id");
  // Not stopped: one that's not running is still there to read.
  await ok("/api/v1/graphs/gph_QuietLog0001");
  await ok(P, {as: ONLY_A});
  const e = await fails(
    "/api/v1/graphs/gph_TrimmedLog01",
    403,
    "api_key_restricted",
    {
      as: ONLY_A,
    },
  );
  assert.equal(e.type, "permission_error");
  // Every endpoint checks.
  for (const sub of [
    "/nodes",
    "/nodes/search?query=depth:0",
    `/nodes/${nodeId(refs[0])}`,
    "/events",
  ]) {
    await fails(
      `/api/v1/graphs/gph_BobsLog00001${sub}`,
      404,
      "resource_missing",
    );
  }
});

test("listing nodes: newest first, or in tree order, every node once", async () => {
  const all = await ok(`${P}/nodes`, {query: {limit: "1000"}});
  assert.deepEqual(Object.keys(all).sort(), required("NodeList"));
  assert.equal(all.data.length, refs.length);
  assert.equal(all.has_more, false);
  assert.equal(all.as_of_event_id, LAST);
  assert.equal(all.url, `${P}/nodes?limit=1000&as_of=${LAST}`);
  const created = all.data.map((n: {created: number}) => n.created);
  assert.deepEqual(
    created,
    [...created].sort((a, b) => b - a),
    "newest first",
  );
  for (const n of all.data) {
    assert.deepEqual(Object.keys(n).sort(), required("Node"));
  }

  const tree = await ok(`${P}/nodes`, {query: {order: "tree", limit: "1000"}});
  const seen = new Set<string>();
  for (const n of tree.data) {
    if (n.parent_id)
      assert.ok(seen.has(n.parent_id), "each parent before its children");
    seen.add(n.id);
  }
  assert.equal(seen.size, refs.length);
  // Default page size: 100 (the examples have fewer).
  assert.equal(
    (await ok(`${P}/nodes`)).data.length,
    Math.min(100, refs.length),
  );
});

test("listing nodes, filtered", async () => {
  const list = async (query: Record<string, string | Array<string>>) =>
    (await ok(`${P}/nodes`, {query: {limit: "1000", ...query}})).data as Array<
      Record<string, any>
    >;
  const parent = someNode(n => n.children.length > 0);
  const kids = await list({parent_id: nodeId(parent)});
  assert.deepEqual(
    kids.map(n => parseNodeId(n.id)).sort(),
    STATE.nodes[parent].children.filter(c => STATE.nodes[c]).sort(),
  );

  const roots = await list({is_root: "true"});
  assert.ok(
    roots.length > 0 && roots.every(n => n.parent_id === null && n.depth === 0),
  );
  const notRoots = await list({is_root: "false"});
  assert.equal(roots.length + notRoots.length, refs.length);

  const root = roots.find(r => r.descendant_count > 1)!;
  const inTree = await list({root_id: root.id});
  assert.equal(inTree.length, root.descendant_count + 1);
  assert.ok(inTree.every(n => n.root_id === root.id));

  // Everything under a node, in tree order, then a level at a time.
  const under = await list({ancestor_id: root.id, order: "tree"});
  assert.equal(under.length, root.descendant_count);
  assert.ok(!under.some(n => n.id === root.id));
  const oneDown = await list({ancestor_id: root.id, max_depth: "1"});
  assert.deepEqual(
    oneDown.map(n => n.id).sort(),
    under
      .filter(n => n.depth === root.depth + 1)
      .map(n => n.id)
      .sort(),
  );

  const session = inTree.find(n => n.kind === "agent")?.session_id ?? root.id;
  assert.ok(
    (await list({session_id: session})).every(n => n.session_id === session),
  );
  assert.ok((await list({kind: "agent"})).every(n => n.kind === "agent"));
  const working = await list({"state[]": ["working", "input_required"]});
  assert.ok(working.length > 0);
  assert.ok(
    working.every(n => ["working", "input_required"].includes(n.state)),
  );
  assert.ok(
    (await list({provider: "claude-code"})).every(
      n => n.provider === "claude-code",
    ),
  );
  const explore = await list({agent_type: "Explore"});
  assert.ok(
    explore.length > 0 && explore.every(n => n.agent_type === "Explore"),
  );
  const blocked = await list({blocked: "true"});
  assert.ok(blocked.length > 0 && blocked.every(n => n.blocked !== null));
  assert.equal(
    blocked.length + (await list({blocked: "false"})).length,
    refs.length,
  );
  const since = EVENTS[60].ms;
  assert.ok(
    (await list({"created[gte]": String(since)})).every(
      n => n.created >= since,
    ),
  );
  assert.ok(
    (await list({"last_event[lt]": String(since)})).every(
      n => n.last_event < since,
    ),
  );

  // Stale: working, waiting on nothing, and silent for 30 minutes, as of
  // when it's asked; a day later, more are.
  const end = EVENTS.at(-1)!.ms;
  const staleNow = await list({stale: "true", as_of: LAST});
  for (const n of staleNow) {
    assert.ok(
      n.state === "working" &&
        n.blocked === null &&
        n.last_event < end - 30 * 60_000,
    );
  }
  const later = await list({stale: "true", as_of: String(end + 86_400_000)});
  assert.ok(later.length > staleNow.length);
  assert.ok(later.every(n => n.state === "working" && n.blocked === null));

  // A node that isn't there has nothing under it.
  assert.deepEqual(await list({ancestor_id: nodeId("x:nobody")}), []);
  const e = await fails(`${P}/nodes`, 400, "parameter_invalid", {
    query: {parent_id: "nope"},
  });
  assert.equal(e.param, "parent_id");
  await fails(`${P}/nodes`, 400, "parameter_invalid", {
    query: {max_depth: "2"},
  });
  await fails(`${P}/nodes`, 400, "parameter_invalid", {
    query: {"state[]": "asleep"},
  });
  await fails(`${P}/nodes`, 400, "parameter_unknown", {query: {fields: "id"}});
});

test("paging through nodes at one moment, while the graph goes on", async () => {
  putLog("PagingLog001", EXAMPLES);
  const path = "/api/v1/graphs/gph_PagingLog001/nodes";
  const first = await ok(path, {query: {limit: "10", order: "tree"}});
  assert.equal(first.has_more, true);
  // A new session starts meanwhile.
  appendText(
    "PagingLog001",
    line(9_000_000, "claude-code:late", "session.started"),
  );
  const seen = [...first.data];
  let page = first;
  while (page.has_more) {
    // The list's url, with the next cursor: as it says to.
    const url = new URL(page.url, "https://site.test");
    url.searchParams.set("starting_after", page.data.at(-1).id);
    page = await ok(`${url.pathname}${url.search}`);
    assert.equal(page.as_of_event_id, first.as_of_event_id);
    seen.push(...page.data);
  }
  assert.equal(seen.length, refs.length, "the late session isn't in it");
  assert.equal(new Set(seen.map(n => n.id)).size, refs.length);
  // Backwards from the last.
  const back = await ok(path, {
    query: {
      limit: "10",
      order: "tree",
      as_of: first.as_of_event_id,
      ending_before: seen.at(-1).id,
    },
  });
  assert.deepEqual(
    back.data.map((n: {id: string}) => n.id),
    seen.slice(-11, -1).map(n => n.id),
  );
  // Now, it's there.
  const now = await ok(path, {query: {limit: "1000"}});
  assert.equal(now.data.length, refs.length + 1);
  await fails(path, 400, "parameter_invalid", {
    query: {starting_after: nodeId("x:nobody")},
  });
});

test("a node: its fields, and whatever's expanded, never an id in an object's place", async () => {
  const child =
    someNode(
      n => Boolean(n.parent) && n.waits.length > 0 && n.tasks.length > 0,
    ) ?? someNode(n => Boolean(n.parent));
  const id = nodeId(child);
  const plain = await ok(`${P}/nodes/${id}`);
  assert.deepEqual(Object.keys(plain).sort(), required("Node"));
  assert.equal(plain.id, id);
  for (const field of [
    "parent",
    "root",
    "session",
    "children",
    "descendants",
    "tasks",
  ]) {
    assert.ok(!(field in plain), `${field} isn't there unless expanded`);
  }
  const expanded = await ok(`${P}/nodes/${id}`, {
    query: {
      "expand[]": [
        "parent",
        "root",
        "session",
        "tasks",
        "spawns",
        "waits",
        "messages",
      ],
    },
  });
  assert.equal(expanded.parent.id, plain.parent_id);
  assert.equal(
    expanded.parent_id,
    plain.parent_id,
    "the id stays beside the object",
  );
  assert.equal(expanded.root.id, plain.root_id);
  assert.equal(expanded.session.id, plain.session_id);
  assert.equal(expanded.tasks.length, STATE.nodes[child].tasks.length);
  assert.ok(
    Array.isArray(expanded.spawns) &&
      Array.isArray(expanded.waits) &&
      Array.isArray(expanded.messages),
  );
  assert.ok(
    !("parent" in expanded.parent) || expanded.parent.parent === undefined,
  );

  // A root has no parent to expand.
  const root = await ok(`${P}/nodes/${plain.root_id}`, {
    query: {"expand[]": "parent"},
  });
  assert.equal(root.parent_id, null);
  assert.ok(!("parent" in root));

  // Children: a list, with a url for the rest, at the same moment.
  const withKids = await ok(`${P}/nodes/${plain.root_id}`, {
    query: {"expand[]": ["children.children", "descendants"]},
  });
  assert.deepEqual(Object.keys(withKids.children).sort(), required("NodeList"));
  assert.equal(
    withKids.children.url,
    `${P}/nodes?parent_id=${plain.root_id}&order=tree&as_of=${LAST}`,
  );
  assert.ok(
    withKids.children.data.every(
      (c: {parent_id: string}) => c.parent_id === plain.root_id,
    ),
  );
  for (const c of withKids.children.data) {
    assert.equal(c.children.object, "list", "two levels");
  }
  // Descendants: everything under it, flat, in tree order.
  const d = withKids.descendants;
  assert.equal(d.data.length, withKids.descendant_count);
  assert.equal(d.has_more, false);
  assert.equal(
    d.url,
    `${P}/nodes?ancestor_id=${plain.root_id}&order=tree&as_of=${LAST}`,
  );
  const placed = new Set([plain.root_id]);
  for (const n of d.data) {
    assert.ok(placed.has(n.parent_id), "each after its parent");
    placed.add(n.id);
  }
  // Its url lists the same.
  const listed = await ok(d.url, {query: {limit: "1000"}});
  assert.deepEqual(
    listed.data.map((n: {id: string}) => n.id),
    d.data.map((n: {id: string}) => n.id),
  );
  const shallow = await ok(`${P}/nodes/${plain.root_id}`, {
    query: {"expand[]": "descendants", descendants_max_depth: "1"},
  });
  assert.equal(shallow.descendants.data.length, shallow.child_count);
  assert.match(shallow.descendants.url, /max_depth=1/);

  // blocked.on: the nodes, beside their ids.
  const waiting = someNode(
    n =>
      Boolean(n.blocked) &&
      !["completed", "failed", "canceled"].includes(n.state),
  );
  const b = await ok(`${P}/nodes/${nodeId(waiting)}`, {
    query: {"expand[]": "blocked.on"},
  });
  assert.deepEqual(
    b.blocked.on.map((n: {id: string}) => n.id),
    b.blocked.on_ids,
  );

  await fails(`${P}/nodes/${id}`, 400, "expand_too_deep", {
    query: {"expand[]": "parent.parent.parent.parent.parent"},
  });
  await fails(`${P}/nodes/${id}`, 400, "expand_invalid", {
    query: {"expand[]": "graph"},
  });
  await fails(`${P}/nodes`, 400, "expand_not_allowed", {
    query: {"expand[]": "data.children"},
  });
  const listed2 = await ok(`${P}/nodes`, {
    query: {"expand[]": "data.parent", limit: "1000"},
  });
  for (const n of listed2.data) {
    assert.equal(n.parent?.id ?? null, n.parent_id);
  }
});

test("a node that isn't there, or isn't yet", async () => {
  const missing = await fails(
    `${P}/nodes/${nodeId("x:nobody")}`,
    404,
    "resource_missing",
  );
  assert.equal(missing.param, "node_id");
  await fails(`${P}/nodes/not-a-node`, 404, "resource_missing");
  // A node made later than as_of: when it's made.
  const late = EVENTS.findIndex((e, i) => i > 10 && e.type === "agent.spawned");
  const e = await fails(
    `${P}/nodes/${nodeId(EVENTS[late].node)}`,
    404,
    "node_not_yet_created",
    {
      query: {as_of: FIRST},
    },
  );
  assert.equal(e.first_event_id, eventId(EVENTS[late].id));
  assert.equal(e.param, "as_of");
});

test("searching nodes: matches, best first, a page at a time at one moment", async () => {
  const s = await ok(`${P}/nodes/search`, {
    query: {query: 'kind:"agent"', limit: "5"},
  });
  assert.deepEqual(Object.keys(s).sort(), required("NodeSearchResult"));
  assert.equal(s.object, "search_result");
  const agents = refs.filter(r => STATE.nodes[r].kind === "agent").length;
  assert.equal(s.total_count, agents);
  assert.equal(s.data.length, 5);
  assert.equal(s.has_more, true);
  assert.ok(s.next_page);
  const lasts = s.data.map((n: {last_event: number}) => n.last_event);
  assert.deepEqual(
    lasts,
    [...lasts].sort((a, b) => b - a),
  );
  const seen = [...s.data];
  let page = s;
  while (page.has_more) {
    page = await ok(`${P}/nodes/search`, {
      query: {query: 'kind:"agent"', limit: "5", page: page.next_page},
    });
    assert.equal(page.as_of_event_id, s.as_of_event_id);
    seen.push(...page.data);
  }
  assert.equal(new Set(seen.map(n => n.id)).size, agents);
  assert.equal(page.next_page, null);

  const both = await ok(`${P}/nodes/search`, {
    query: {
      query: 'state:"working" AND descendant_count>0',
      "expand[]": "data.root",
    },
  });
  for (const n of both.data) {
    assert.equal(n.state, "working");
    assert.ok(n.descendant_count > 0);
    assert.equal(n.root.id, n.root_id);
  }
  const e = await fails(`${P}/nodes/search`, 400, "search_query_invalid", {
    query: {query: "state"},
  });
  assert.equal(e.param, "query");
  await fails(`${P}/nodes/search`, 400, "parameter_missing");
  await fails(`${P}/nodes/search`, 400, "parameter_invalid", {
    query: {query: "depth:0", page: "junk"},
  });
});

test("events: newest first, or oldest, each with what it changed", async () => {
  const list = await ok(`${P}/events`, {query: {limit: "1000"}});
  assert.deepEqual(Object.keys(list).sort(), required("EventList"));
  assert.equal(list.data.length, EVENTS.length);
  assert.equal(list.data[0].id, LAST);
  assert.equal(list.as_of_event_id, LAST);
  for (const e of list.data) {
    assert.deepEqual(Object.keys(e).sort(), required("Event"));
    assert.deepEqual(Object.keys(e.source).sort(), required("EventSource"));
    for (const c of e.changes) {
      assert.deepEqual(Object.keys(c).sort(), required("NodeChange"));
    }
  }
  const asc = await ok(`${P}/events`, {query: {order: "asc", limit: "3"}});
  assert.deepEqual(
    asc.data.map((e: {id: string}) => e.id),
    EVENTS.slice(0, 3).map(e => eventId(e.id)),
  );
  // The first event made its node.
  const made = asc.data[0];
  assert.equal(made.type, "session.started");
  assert.equal(made.node_id, nodeId(EVENTS[0].node));
  assert.equal(made.changes[0].created, true);
  assert.equal(made.changes[0].fields.kind.to, "session");
  assert.deepEqual(made.payload, EVENTS[0].data);
  assert.equal(made.created, EVENTS[0].ms);

  // Following a graph: what's happened since an event.
  const since = await ok(`${P}/events`, {
    query: {order: "asc", starting_after: eventId(EVENTS[100].id)},
  });
  assert.equal(since.data.length, EVENTS.length - 101);

  // Filtered.
  const spawned = await ok(`${P}/events`, {
    query: {"type[]": "agent.*", limit: "1000"},
  });
  assert.ok(spawned.data.length > 0);
  assert.ok(
    spawned.data.every((e: {type: string}) => e.type.startsWith("agent.")),
  );
  const two = await ok(`${P}/events`, {
    query: {"type[]": ["status", "session.started"], limit: "1000"},
  });
  assert.ok(
    two.data.every((e: {type: string}) =>
      ["status", "session.started"].includes(e.type),
    ),
  );
  const one = EVENTS[50].node;
  const ofOne = await ok(`${P}/events`, {
    query: {node_id: nodeId(one), limit: "1000"},
  });
  assert.equal(ofOne.data.length, EVENTS.filter(e => e.node === one).length);
  const root = STATE.roots.find(r => STATE.nodes[r].children.length)!;
  const ofTree = await ok(`${P}/events`, {
    query: {root_id: nodeId(root), limit: "1000"},
  });
  assert.ok(
    ofTree.data.length >
      ofTree.data.filter((e: {node_id: string}) => e.node_id === nodeId(root))
        .length,
  );
  const window = await ok(`${P}/events`, {
    query: {
      "created[gte]": String(EVENTS[10].ms),
      "created[lte]": String(EVENTS[20].ms),
      limit: "1000",
    },
  });
  assert.ok(
    window.data.every(
      (e: {created: number}) =>
        e.created >= EVENTS[10].ms && e.created <= EVENTS[20].ms,
    ),
  );
  await fails(`${P}/events`, 400, "parameter_invalid", {
    query: {"type[]": "agent spawned"},
  });
  await fails(`${P}/events`, 400, "parameter_unknown", {query: {as_of: FIRST}});

  // A follower that fell behind the graph's retained history is told so,
  // and where it now starts.
  const behind = await fails(
    "/api/v1/graphs/gph_TrimmedLog01/events",
    410,
    "outside_retention_window",
    {query: {order: "asc", starting_after: FIRST}},
  );
  assert.equal(behind.param, "starting_after");
  const kept = await ok("/api/v1/graphs/gph_TrimmedLog01", {
    query: {as_of: LAST},
  });
  assert.equal(behind.first_event_id, kept.retention.first_event_id);
  // One that's held, but not in this list, is from another list.
  await fails(`${P}/events`, 400, "parameter_invalid", {
    query: {"type[]": "agent.*", starting_after: FIRST},
  });
});

test("an event, and its node just after it", async () => {
  const i = EVENTS.findIndex(e => e.type === "agent.spawned");
  const id = eventId(EVENTS[i].id);
  const e = await get(`${P}/events/${id}`, {query: {"expand[]": "node"}});
  assert.equal(e.status, 200);
  assert.equal(
    e.headers.get("cache-control"),
    "private, max-age=31536000, immutable",
  );
  assert.equal(e.body.id, id);
  assert.equal(e.body.node.id, e.body.node_id);
  assert.equal(e.body.node.as_of_event_id, id);
  const asOf = await ok(`${P}/nodes/${e.body.node_id}`, {query: {as_of: id}});
  const {stale: _a, ...one} = e.body.node;
  const {stale: _b, ...other} = asOf;
  assert.deepEqual(one, other, "the same as the node as of it");
  // Its node's parent, as of then.
  const deep = await ok(`${P}/events/${id}`, {
    query: {"expand[]": "node.parent"},
  });
  assert.equal(deep.node.parent?.id ?? null, deep.node.parent_id);
  const listed = await ok(`${P}/events`, {
    query: {"expand[]": "data.node", limit: "5"},
  });
  assert.ok(
    listed.data.every(
      (x: {node: {id: string}; node_id: string}) => x.node.id === x.node_id,
    ),
  );

  // Expanded past the node, a page's nodes are each as of their own event.
  const page = await ok(`${P}/events`, {
    query: {
      "expand[]": "data.node.parent",
      order: "asc",
      starting_after: eventId(EVENTS[40].id),
      limit: "30",
    },
  });
  assert.equal(page.data.length, 30);
  limiter = new RateLimiter(1000, 1000, () => clock);
  for (const x of page.data) {
    const alone = await ok(`${P}/events/${x.id}`, {
      query: {"expand[]": "node.parent"},
    });
    assert.deepEqual(x.node, alone.node, x.id);
  }
  // (So many requests at once are past the rate limit's burst.)
  limiter = new RateLimiter(25, 100, () => clock);

  const missing = await fails(
    `${P}/events/evt_ZZZZZZZZZZZZZZZZZZZZZZZZZZ`,
    404,
    "resource_missing",
  );
  assert.equal(missing.param, "event_id");
  await fails(`${P}/events/nope`, 404, "resource_missing");
  const old = await fails(
    `/api/v1/graphs/gph_TrimmedLog01/events/${FIRST}`,
    410,
    "outside_retention_window",
  );
  assert.equal(old.param, "event_id");
  await fails(`${P}/events/${id}`, 400, "expand_invalid", {
    query: {"expand[]": "payload"},
  });
});

test("an ETag: unchanged, a 304 that doesn't count against the rate limit", async () => {
  const first = await get(`${P}/nodes`);
  const etag = first.headers.get("etag")!;
  assert.match(etag, /^W\/".+"$/);
  const left = Number(first.headers.get("ratelimit-remaining"));
  const again = await get(`${P}/nodes`, {headers: {"if-none-match": etag}});
  assert.equal(again.status, 304);
  assert.equal(again.body, null);
  assert.equal(again.headers.get("etag"), etag);
  const after = await get(`${P}/nodes`);
  // The 304 was given back: one request taken since (this one), not two.
  assert.ok(Number(after.headers.get("ratelimit-remaining")) >= left - 1);
  // Changed, it's sent again.
  appendText(
    "ExampleLog01",
    line(9_000_001, "claude-code:newer", "session.started"),
  );
  const changed = await get(`${P}/nodes`, {headers: {"if-none-match": etag}});
  assert.equal(changed.status, 200);
  assert.notEqual(changed.headers.get("etag"), etag);
});

test("rate limits: past the burst, 429 with when to try again", async () => {
  const saved = limiter;
  limiter = new RateLimiter(1, 3, () => clock);
  try {
    for (let i = 0; i < 3; i++) {
      const res = await get("/api/v1/graphs");
      assert.equal(res.status, 200);
      assert.equal(res.headers.get("ratelimit-limit"), "1");
      assert.equal(res.headers.get("ratelimit-remaining"), String(2 - i));
    }
    const e = await fails("/api/v1/graphs", 429, "rate_limit");
    assert.equal(e.type, "rate_limit_error");
    assert.equal((await get("/api/v1/graphs")).headers.get("retry-after"), "1");
    // Another key isn't held back.
    assert.equal((await get("/api/v1/graphs", {as: BOB})).status, 200);
    clock += 1000;
    assert.equal((await get("/api/v1/graphs")).status, 200);
  } finally {
    limiter = saved;
  }
});

test("a log is read once, then only what's new; trimmed, it's read again from its keyframe", async () => {
  graphs = new Graphs(readChunks, () => clock);
  putLog("ReadingLog01", EXAMPLES, {chunkSize: 2000});
  const path = "/api/v1/graphs/gph_ReadingLog01";
  reads = 0;
  const g1 = await ok(path);
  const whole = reads;
  assert.ok(whole > 2, "paged");
  reads = 0;
  await ok(path);
  assert.equal(reads, 1, "one read, finding nothing new");
  appendText(
    "ReadingLog01",
    line(9_000_002, "claude-code:newest", "session.started"),
  );
  const g2 = await ok(path);
  assert.equal(g2.counts.nodes, g1.counts.nodes + 1);
  assert.equal(g2.retention.event_count, g1.retention.event_count + 1);

  // A live share trims its start once it's sent a keyframe: read whole
  // again (a while later), it starts there.
  const cut = trimmed(EXAMPLES, 40);
  putLog("ReadingLog01", "");
  const log = store.get("ReadingLog01")!;
  log.chunks.set(1_000_000, cut);
  clock += 11 * 60 * 1000;
  const g3 = await ok(path);
  assert.equal(g3.counts.nodes, g1.counts.nodes);
  assert.ok(g3.retention.event_count < g1.retention.event_count);

  // Trimmed while it's read: it reads again from where what's left starts.
  putLog("GapLog000001", EXAMPLES, {chunkSize: 1000});
  const before = [...store.get("GapLog000001")!.chunks.keys()];
  const reading = new Graphs(
    async (id, after) => {
      const r = await readChunks(id, after);
      // After the first read, the log's start goes, but for a keyframe.
      if (!after) {
        trimBefore(id, Infinity);
        store.get(id)!.chunks.set(before.at(-1)! + 100_000, cut);
      }
      return r;
    },
    () => clock,
  );
  const loaded = await reading.get("GapLog000001");
  assert.ok(loaded.base, "it starts from the keyframe");
  assert.equal(loaded.events.length, g3.retention.event_count);
});

test("a log read whole again leaves what's answering from it as it was", async () => {
  putLog("RereadLog001", EXAMPLES, {chunkSize: 2000});
  let hold: Promise<void> | null = null;
  const reading = new Graphs(
    async (id, after) => {
      await hold;
      return readChunks(id, after);
    },
    () => clock,
  );
  const was = await reading.get("RereadLog001");
  const last = was.events.length - 1;
  const nodes = was.stateAt(last).nodes;
  // A while later, another request reads it whole again, and is held up
  // reading; one still answering from what it had carries on.
  let release = () => {};
  hold = new Promise(resolve => (release = resolve));
  clock += 11 * 60 * 1000;
  const again = reading.get("RereadLog001");
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(
    Object.keys(was.stateAt(last, clock).nodes).sort(),
    Object.keys(nodes).sort(),
  );
  assert.ok(was.appliedAt([last], G)[0].changes.length);
  release();
  const now = await again;
  assert.notEqual(now, was);
  assert.equal(now.events.length, was.events.length);
});

test("something unexpected: a 500 in the API's shape, saying nothing more", async () => {
  const saved = console.error;
  const logged: Array<unknown> = [];
  console.error = (...args: Array<unknown>) => logged.push(args);
  try {
    const res = await serve(
      new Request("https://site.test/api/v1/graphs", {
        headers: {authorization: `Bearer ${ALICE}`},
      }),
      async () => {
        throw new Error("the database is on fire");
      },
      {keyOf: async (s: string) => keys.get(s) ?? null, limiter},
    );
    assert.equal(res.status, 500);
    const body = await res.json();
    assert.equal(body.error.type, "api_error");
    assert.doesNotMatch(body.error.message, /fire/);
    assert.equal(res.headers.get("cache-control"), "no-store");
    assert.equal(logged.length, 1, "logged, with its request id");
  } finally {
    console.error = saved;
  }
});
