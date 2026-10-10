// The graph API's endpoints (site/openapi.json): what each answers, given
// a request already let through (lib/api/handler.ts). Storage is passed in
// (`V1Deps`), so they're tested without it (tests/api-v1.test.mts).

import {type Applied} from "./changes";
import {
  ApiError,
  apiKeyRestricted,
  notYetCreated,
  outsideRetention,
  parameterInvalid,
  parameterMissing,
  resourceMissing,
} from "./errors";
import {parseExpand, renderNode, type RenderAt, type Trie} from "./expand";
import type {Ctx, Reply} from "./handler";
import {
  eventId,
  graphId,
  nodeId,
  parseEventId,
  parseGraphId,
  parseNodeId,
} from "./ids";
import {
  type ApiNode,
  apiNode,
  byNewest,
  counts,
  createdOf,
  STATES,
  type Tree,
} from "./model";
import {paginate} from "./paginate";
import {type AsOf, inRange, LIST_PARAMS, parseQuery} from "./query";
import type {LogEvent} from "./reducer";
import {matches, parseSearch} from "./search";
import type {Graphs, Loaded} from "./source";
import type {Meta, Share} from "../store";

/** What the endpoints read, passed in. */
export type V1Deps = {
  sharesOf: (uid: string) => Promise<Array<Share>>;
  getMeta: (id: string) => Promise<Meta | null>;
  graphs: Graphs;
  now: () => number;
};

/** The API's path to graph `log`. */
const pathOf = (log: string) => `/api/v1/graphs/${graphId(log)}`;

/** A graph the key can read: its log's metadata, and the log, loaded. */
async function open(
  ctx: Ctx,
  deps: V1Deps,
  param: string,
): Promise<{log: string; meta: Meta; loaded: Loaded}> {
  const log = parseGraphId(param);
  if (!log) {
    throw resourceMissing("graph", param, "graph_id");
  }
  const meta = await deps.getMeta(log);
  // Another account's log, or one that isn't a live share, isn't there for
  // this key: not even to say it exists.
  if (!meta || !meta.owner || meta.owner !== ctx.key.uid) {
    throw resourceMissing("graph", graphId(log), "graph_id");
  }
  if (ctx.key.kind === "restricted" && !ctx.key.graphs?.includes(log)) {
    throw apiKeyRestricted(graphId(log));
  }
  return {log, meta, loaded: await deps.graphs.get(log)};
}

/** A point in a graph's history: the event it's after, and when to judge staleness. */
type Point = {
  /** The event's place in the log; -1 before any. */
  index: number;
  /** Its id, as the API names it. */
  event: string | null;
  /** When it's judged; for an event, its own time (undefined). */
  nowMs?: number;
  /** Whether it's a fixed event, so what's said of it never changes. */
  fixed: boolean;
};

/** Where `as_of` is (or now, without one); refused if the log doesn't hold it. */
function pointOf(loaded: Loaded, asOf: AsOf | undefined, now: number): Point {
  const {events} = loaded;
  const last = events.length - 1;
  if (!asOf) {
    // To the second, so requests in the same second share a state.
    return {
      index: last,
      event: last >= 0 ? eventId(events[last].id) : null,
      nowMs: Math.floor(now / 1000) * 1000,
      fixed: false,
    };
  }
  if (!events.length) {
    throw new ApiError(
      410,
      "invalid_request_error",
      "outside_retention_window",
      "This graph holds no events yet.",
      "as_of",
    );
  }
  const first = eventId(events[0].id);
  const latest = eventId(events[last].id);
  if ("event" in asOf) {
    const i = loaded.index.get(asOf.event);
    if (i === undefined) {
      if (asOf.event < events[0].id) {
        throw outsideRetention(first, latest);
      }
      throw resourceMissing("event", eventId(asOf.event), "as_of");
    }
    return {index: i, event: eventId(events[i].id), fixed: true};
  }
  if (asOf.time < events[0].ms) {
    throw outsideRetention(first, latest);
  }
  // The last event at or before it.
  let lo = 0;
  let hi = last;
  while (lo < hi) {
    const mid = Math.ceil((lo + hi) / 2);
    if (events[mid].ms <= asOf.time) {
      lo = mid;
    } else {
      hi = mid - 1;
    }
  }
  return {
    index: lo,
    event: eventId(events[lo].id),
    nowMs: asOf.time,
    fixed: false,
  };
}

const treeAt = (loaded: Loaded, point: Point) =>
  loaded.stateAt(point.index, point.nowMs);

/** A list's own URL: its path, with what defined it, and the moment it's of. */
function listUrl(ctx: Ctx, path: string, asOf: string | null): string {
  const params = new URLSearchParams(ctx.url.searchParams);
  params.delete("starting_after");
  params.delete("ending_before");
  params.delete("page");
  if (asOf) {
    params.set("as_of", asOf);
  }
  const query = params.toString();
  return query ? `${path}?${query}` : path;
}

/** A graph, as the API shows it. */
function graphObject(
  ctx: Ctx,
  log: string,
  meta: Meta,
  loaded: Loaded,
  point: Point,
) {
  const {events} = loaded;
  const first = events[0];
  const last = events.at(-1);
  const created = meta.createdAt.toMillis();
  return {
    id: graphId(log),
    object: "graph" as const,
    as_of_event_id: point.event,
    source: meta.source,
    url: `${ctx.origin}/l/${log}`,
    created,
    last_event: last?.ms ?? created,
    retention: {
      first_event_id: first ? eventId(first.id) : null,
      last_event_id: last ? eventId(last.id) : null,
      first_event_created: first?.ms ?? null,
      last_event_created: last?.ms ?? null,
      event_count: events.length,
    },
    counts: counts(treeAt(loaded, point)),
  };
}

const GRAPH_LIST_PARAMS = {
  ...LIST_PARAMS,
  last_event: {kind: "range"},
} as const;

/** GET /v1/graphs */
export async function listGraphs(ctx: Ctx, deps: V1Deps): Promise<Reply> {
  const q = parseQuery(ctx.url, GRAPH_LIST_PARAMS);
  let shares = await deps.sharesOf(ctx.key.uid);
  if (ctx.key.kind === "restricted") {
    shares = shares.filter(s => ctx.key.graphs?.includes(s.id));
  }
  const now = deps.now();
  const graphs = [];
  for (const share of shares) {
    const meta = await deps.getMeta(share.id);
    if (!meta || meta.owner !== ctx.key.uid) {
      continue;
    }
    const loaded = await deps.graphs.get(share.id);
    graphs.push(
      graphObject(ctx, share.id, meta, loaded, pointOf(loaded, undefined, now)),
    );
  }
  const listed = graphs
    .filter(g => inRange(g.last_event, q.last_event))
    .sort((a, b) => b.last_event - a.last_event || (a.id < b.id ? -1 : 1));
  const page = paginate(listed, g => g.id, q);
  return {
    body: {
      object: "list",
      url: listUrl(ctx, "/api/v1/graphs", null),
      as_of_event_id: null,
      ...page,
    },
  };
}

/** GET /v1/graphs/{graph_id} */
export async function retrieveGraph(
  ctx: Ctx,
  deps: V1Deps,
  graph: string,
): Promise<Reply> {
  const q = parseQuery(ctx.url, {as_of: {kind: "asOf"}});
  const {log, meta, loaded} = await open(ctx, deps, graph);
  const point = pointOf(loaded, q.as_of, deps.now());
  return {
    body: graphObject(ctx, log, meta, loaded, point),
    immutable: point.fixed,
  };
}

/** A node id parameter's node; refused if it isn't one. */
function nodeParam(param: string, value: string | undefined): string | null {
  if (value === undefined) {
    return null;
  }
  const ref = parseNodeId(value);
  if (!ref) {
    throw parameterInvalid(
      param,
      `Invalid ${param}: ${value} isn't a node id.`,
    );
  }
  return ref;
}

const NODE_LIST_PARAMS = {
  ...LIST_PARAMS,
  as_of: {kind: "asOf"},
  expand: {kind: "strings"},
  parent_id: {kind: "string", max: 1000},
  ancestor_id: {kind: "string", max: 1000},
  max_depth: {kind: "int", min: 1},
  root_id: {kind: "string", max: 1000},
  session_id: {kind: "string", max: 1000},
  is_root: {kind: "bool"},
  kind: {kind: "enum", values: ["session", "agent"]},
  state: {kind: "enums", values: STATES},
  provider: {kind: "string", max: 200},
  agent_type: {kind: "string", max: 200},
  stale: {kind: "bool"},
  blocked: {kind: "bool"},
  created: {kind: "range"},
  last_event: {kind: "range"},
  order: {kind: "enum", values: ["newest", "tree"]},
} as const;

/** GET /v1/graphs/{graph_id}/nodes */
export async function listNodes(
  ctx: Ctx,
  deps: V1Deps,
  graph: string,
): Promise<Reply> {
  const q = parseQuery(ctx.url, NODE_LIST_PARAMS);
  const trie = parseExpand(q.expand, "nodes");
  const parent = nodeParam("parent_id", q.parent_id);
  const ancestor = nodeParam("ancestor_id", q.ancestor_id);
  const root = nodeParam("root_id", q.root_id);
  const session = nodeParam("session_id", q.session_id);
  if (q.max_depth !== undefined && ancestor === null) {
    throw parameterInvalid("max_depth", "max_depth needs ancestor_id.");
  }
  const {log, loaded} = await open(ctx, deps, graph);
  const point = pointOf(loaded, q.as_of, deps.now());
  const tree = treeAt(loaded, point);
  const order = q.order ?? "newest";

  let refs: Array<string>;
  if (ancestor !== null) {
    refs = tree.has(ancestor) ? tree.under(ancestor, q.max_depth) : [];
  } else {
    refs = tree.treeOrder();
  }
  if (order === "newest") {
    refs = [...refs].sort((a, b) => byNewest(tree.node(a), tree.node(b)));
  }
  const states = q.state ? new Set(q.state) : null;
  const kept = refs.filter(ref => {
    const n = tree.node(ref);
    const p = tree.parentOf(ref);
    return (
      (parent === null || p === parent) &&
      (root === null || tree.rootOf(ref) === root) &&
      (session === null || tree.sessionOf(ref) === session) &&
      (q.is_root === undefined || (p === null) === q.is_root) &&
      (q.kind === undefined || n.kind === q.kind) &&
      (!states || states.has(n.state)) &&
      (q.provider === undefined || n.provider === q.provider) &&
      (q.agent_type === undefined || n.agent_type === q.agent_type) &&
      (q.stale === undefined || n.stale === q.stale) &&
      (q.blocked === undefined ||
        (apiNode(tree, ref, "", null).blocked !== null) === q.blocked) &&
      inRange(createdOf(n), q.created) &&
      inRange(Date.parse(n.last_event_at), q.last_event)
    );
  });
  const page = paginate(kept, nodeId, q);
  const at = renderAt(log, tree, point);
  return {
    body: {
      object: "list",
      url: listUrl(ctx, `${pathOf(log)}/nodes`, point.event),
      as_of_event_id: point.event,
      has_more: page.has_more,
      data: page.data.map(ref => renderNode(at, ref, trie)),
    },
    immutable: point.fixed,
  };
}

function renderAt(
  log: string,
  tree: Tree,
  point: Point,
  maxDepth?: number,
): RenderAt {
  return {
    tree,
    graphId: graphId(log),
    asOf: point.event,
    base: pathOf(log),
    ...(maxDepth !== undefined ? {maxDepth} : {}),
  };
}

/** A search's page cursor: where the next page starts, and the moment it's all of. */
type PageCursor = {o: number; e: string | null; t?: number};

function readPage(page: string): PageCursor {
  try {
    const c = JSON.parse(Buffer.from(page, "base64url").toString("utf8"));
    if (
      typeof c?.o === "number" &&
      c.o >= 0 &&
      (c.e === null || typeof c.e === "string") &&
      (c.t === undefined || typeof c.t === "number")
    ) {
      return c as PageCursor;
    }
  } catch {
    // Not one: refused below.
  }
  throw parameterInvalid(
    "page",
    "Invalid page: pass a next_page as it was given.",
  );
}

const writePage = (c: PageCursor) =>
  Buffer.from(JSON.stringify(c), "utf8").toString("base64url");

/** GET /v1/graphs/{graph_id}/nodes/search */
export async function searchNodes(
  ctx: Ctx,
  deps: V1Deps,
  graph: string,
): Promise<Reply> {
  const q = parseQuery(ctx.url, {
    query: {kind: "string", max: 2000},
    as_of: {kind: "asOf"},
    expand: {kind: "strings"},
    limit: LIST_PARAMS.limit,
    page: {kind: "string", max: 1000},
  });
  if (q.query === undefined) {
    throw parameterMissing("query");
  }
  const query = parseSearch(q.query);
  const trie = parseExpand(q.expand, "nodes");
  const cursor = q.page !== undefined ? readPage(q.page) : null;
  const {log, loaded} = await open(ctx, deps, graph);
  let point: Point;
  if (cursor) {
    // The moment the first page was of.
    const index = cursor.e
      ? loaded.index.get(parseEventId(cursor.e) ?? "")
      : -1;
    if (index === undefined) {
      throw parameterInvalid(
        "page",
        "That page's moment is no longer in this graph's history.",
      );
    }
    point = {
      index,
      event: cursor.e,
      ...(cursor.t !== undefined ? {nowMs: cursor.t} : {}),
      fixed: cursor.t === undefined,
    };
  } else {
    point = pointOf(loaded, q.as_of, deps.now());
  }
  const tree = treeAt(loaded, point);
  const found: Array<ApiNode> = tree
    .treeOrder()
    .map(ref => apiNode(tree, ref, graphId(log), point.event))
    .filter(n => matches(query, n))
    .sort((a, b) => b.last_event - a.last_event || (a.id < b.id ? -1 : 1));
  const limit = q.limit ?? 100;
  const start = cursor?.o ?? 0;
  const data = found.slice(start, start + limit);
  const hasMore = start + limit < found.length;
  const at = renderAt(log, tree, point);
  const params = new URLSearchParams({query: q.query});
  return {
    body: {
      object: "search_result",
      url: `${pathOf(log)}/nodes/search?${params}`,
      as_of_event_id: point.event,
      has_more: hasMore,
      next_page: hasMore
        ? writePage({
            o: start + limit,
            e: point.event,
            ...(point.nowMs !== undefined ? {t: point.nowMs} : {}),
          })
        : null,
      total_count: found.length,
      data: data.map(n => renderNode(at, n.provider_ref, trie)),
    },
    immutable: point.fixed,
  };
}

/** GET /v1/graphs/{graph_id}/nodes/{node_id} */
export async function retrieveNode(
  ctx: Ctx,
  deps: V1Deps,
  graph: string,
  node: string,
): Promise<Reply> {
  const q = parseQuery(ctx.url, {
    as_of: {kind: "asOf"},
    expand: {kind: "strings"},
    descendants_max_depth: {kind: "int", min: 1},
  });
  const trie = parseExpand(q.expand, "node");
  const ref = parseNodeId(node);
  if (!ref) {
    throw resourceMissing("node", node, "node_id");
  }
  const {log, loaded} = await open(ctx, deps, graph);
  const point = pointOf(loaded, q.as_of, deps.now());
  const tree = treeAt(loaded, point);
  if (!tree.has(ref)) {
    // There later? Then it's when that's asked of, not which.
    const made = loaded.events.findIndex(
      (e, i) => i > point.index && e.node === ref,
    );
    if (made >= 0 && q.as_of) {
      throw notYetCreated(node, eventId(loaded.events[made].id));
    }
    throw resourceMissing("node", node, "node_id");
  }
  return {
    body: renderNode(
      renderAt(log, tree, point, q.descendants_max_depth),
      ref,
      trie,
    ),
    immutable: point.fixed,
  };
}

/** An event, as the API shows it. */
function eventObject(log: string, e: LogEvent, applied: Applied) {
  return {
    id: eventId(e.id),
    object: "event" as const,
    type: e.type,
    created: e.ms,
    graph_id: graphId(log),
    node_id: nodeId(e.node),
    source: {
      provider: e.source?.provider ?? e.node.split(":")[0],
      provider_version: e.source?.provider_version ?? null,
      adapter: e.source?.adapter ?? null,
    },
    payload: e.data ?? {},
    changes: applied.changes,
  };
}

/** An event's node, as `expand[]=node` (or `data.node`) asks. */
function eventNode(
  log: string,
  loaded: Loaded,
  index: number,
  applied: Applied,
  trie: Trie,
  trees?: Map<number, Tree>,
): Record<string, unknown> | null {
  const e = loaded.events[index];
  const id = eventId(e.id);
  if (!trie.size && applied.node) {
    return {...applied.node, as_of_event_id: id};
  }
  const tree = trees?.get(index) ?? loaded.stateAt(index);
  if (!tree.has(e.node)) {
    return null;
  }
  return renderNode(
    renderAt(log, tree, {index, event: id, fixed: true}),
    e.node,
    trie,
  );
}

/** `type[]`: an event type, or a family of them (`agent.*`). */
function typeMatcher(types: ReadonlyArray<string> | undefined) {
  if (!types?.length) {
    return () => true;
  }
  for (const t of types) {
    if (!/^([a-z_]+(\.[a-z_]+)*(\.\*)?|\*)$/.test(t)) {
      throw parameterInvalid("type", `Invalid type: ${t}.`);
    }
  }
  return (type: string) =>
    types.some(t =>
      t === "*"
        ? true
        : t.endsWith(".*")
          ? type.startsWith(t.slice(0, -1))
          : type === t,
    );
}

/** GET /v1/graphs/{graph_id}/events */
export async function listEvents(
  ctx: Ctx,
  deps: V1Deps,
  graph: string,
): Promise<Reply> {
  const q = parseQuery(ctx.url, {
    ...LIST_PARAMS,
    type: {kind: "strings", max: 50},
    node_id: {kind: "string", max: 1000},
    root_id: {kind: "string", max: 1000},
    created: {kind: "range"},
    order: {kind: "enum", values: ["desc", "asc"]},
    expand: {kind: "strings"},
  });
  const trie = parseExpand(q.expand, "events");
  const node = nodeParam("node_id", q.node_id);
  const root = nodeParam("root_id", q.root_id);
  const isType = typeMatcher(q.type);
  const {log, loaded} = await open(ctx, deps, graph);
  const {events} = loaded;
  let inTree: Set<string> | null = null;
  if (root !== null) {
    const tree = loaded.stateAt(events.length - 1);
    inTree = new Set(tree.has(root) ? [root, ...tree.under(root)] : []);
  }
  const indexes = events
    .map((_, i) => i)
    .filter(i => {
      const e = events[i];
      return (
        isType(e.type) &&
        (node === null || e.node === node) &&
        (!inTree || inTree.has(e.node)) &&
        inRange(e.ms, q.created)
      );
    });
  if ((q.order ?? "desc") === "desc") {
    indexes.reverse();
  }
  // A cursor the log's start has since been trimmed past (a follower that
  // fell behind) is out of its history, not from another list.
  for (const param of ["starting_after", "ending_before"] as const) {
    const cursor = q[param] !== undefined ? parseEventId(q[param]) : null;
    if (
      cursor &&
      events.length &&
      !loaded.index.has(cursor) &&
      cursor < events[0].id
    ) {
      throw outsideRetention(
        eventId(events[0].id),
        eventId(events[events.length - 1].id),
        param,
      );
    }
  }
  const page = paginate(indexes, i => eventId(events[i].id), q);
  const applied = loaded.appliedAt(page.data, graphId(log));
  const nodeTrie = trie.get("node");
  // Expanding past the node needs each event's whole graph: all from one
  // replay, as one each would be a replay of the log per event.
  const trees = nodeTrie?.size ? loaded.treesAfter(page.data) : undefined;
  const data = page.data.map((i, k) => ({
    ...eventObject(log, events[i], applied[k]),
    ...(nodeTrie
      ? {
          node:
            eventNode(log, loaded, i, applied[k], nodeTrie, trees) ?? undefined,
        }
      : {}),
  }));
  const head = events.length ? eventId(events[events.length - 1].id) : null;
  return {
    body: {
      object: "list",
      url: listUrl(ctx, `${pathOf(log)}/events`, null),
      as_of_event_id: head,
      has_more: page.has_more,
      data,
    },
  };
}

/** GET /v1/graphs/{graph_id}/events/{event_id} */
export async function retrieveEvent(
  ctx: Ctx,
  deps: V1Deps,
  graph: string,
  event: string,
): Promise<Reply> {
  const q = parseQuery(ctx.url, {expand: {kind: "strings"}});
  const trie = parseExpand(q.expand, "event");
  const id = parseEventId(event);
  if (!id) {
    throw resourceMissing("event", event, "event_id");
  }
  const {log, loaded} = await open(ctx, deps, graph);
  const {events} = loaded;
  const i = loaded.index.get(id);
  if (i === undefined) {
    if (events.length && id < events[0].id) {
      throw outsideRetention(
        eventId(events[0].id),
        eventId(events[events.length - 1].id),
        "event_id",
      );
    }
    throw resourceMissing("event", event, "event_id");
  }
  const [applied] = loaded.appliedAt([i], graphId(log));
  const nodeTrie = trie.get("node");
  return {
    body: {
      ...eventObject(log, events[i], applied),
      ...(nodeTrie
        ? {node: eventNode(log, loaded, i, applied, nodeTrie) ?? undefined}
        : {}),
    },
    immutable: true,
  };
}
