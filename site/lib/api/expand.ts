// Expanding (site/openapi.json, "Ids versus objects, and expanding"): a
// field that refers to another object is always its id (`parent_id`); the
// object itself (`parent`) is there only when asked for, with `expand[]`.
// Paths follow fields with dots (`children.children`, `blocked.on`), at
// most 4 deep; in a list, each item's, after `data.`.

import {expandError} from "./errors";
import {nodeId} from "./ids";
import {
  apiMessages,
  apiNode,
  apiSpawns,
  apiTasks,
  apiWaits,
  type ApiNode,
  type Tree,
} from "./model";

/** Paths asked for, as a tree of field names. */
export type Trie = Map<string, Trie>;

/** Where expanding is asked for: what the paths start from. */
export type ExpandAt =
  | "node" // retrieving a node
  | "nodes" // listing or searching nodes (data.…)
  | "event" // retrieving an event
  | "events"; // listing events (data.…)

export const MOST_PATHS = 20;
export const DEEPEST = 4;

/** How many of a node's children `children` holds, and how many nodes `descendants` does. */
export const CHILDREN_SHOWN = 100;
export const DESCENDANTS_SHOWN = 1000;

const NODE_REFS = ["parent", "root", "session", "requested_by"];
const NODE_LEAVES = ["tasks", "spawns", "waits", "messages"];

/** Reads `expand[]` for `at`; an error naming what's wrong with any path. */
export function parseExpand(
  paths: ReadonlyArray<string> | undefined,
  at: ExpandAt,
): Trie {
  const trie: Trie = new Map();
  if (!paths?.length) {
    return trie;
  }
  if (paths.length > MOST_PATHS) {
    throw expandError(
      "parameter_invalid",
      `Too many paths to expand: at most ${MOST_PATHS}.`,
    );
  }
  const inList = at === "nodes" || at === "events";
  for (const path of paths) {
    let segments = path.split(".");
    if (!segments.every(s => /^[a-z_]+$/.test(s))) {
      throw expandError(
        "expand_invalid",
        `Can't expand ${JSON.stringify(path)}.`,
      );
    }
    if (inList) {
      if (segments[0] !== "data" || segments.length < 2) {
        throw expandError(
          "expand_invalid",
          `In a list, expand each item's fields after data.: data.${path}.`,
        );
      }
      segments = segments.slice(1);
    }
    if (segments.length > DEEPEST) {
      throw expandError(
        "expand_too_deep",
        `The path ${path} goes ${segments.length} levels deep; at most ${DEEPEST} are allowed.`,
      );
    }
    check(
      path,
      segments,
      at === "event" || at === "events" ? "event" : "node",
      inList,
    );
    let node = trie;
    for (const s of segments) {
      if (!node.has(s)) {
        node.set(s, new Map());
      }
      node = node.get(s)!;
    }
  }
  return trie;
}

/** Refuses a path that names something that can't be expanded where it is. */
function check(
  path: string,
  segments: ReadonlyArray<string>,
  start: "node" | "event",
  inList: boolean,
): void {
  let type = start as "node" | "event" | "blocked" | "leaf";
  const bad = (why: string): never => {
    throw expandError("expand_invalid", `Can't expand ${path}: ${why}`);
  };
  for (const [i, s] of segments.entries()) {
    if (type === "event") {
      if (s !== "node") {
        bad(`an event's ${s} isn't expandable (node is).`);
      }
      type = "node";
    } else if (type === "blocked") {
      if (s !== "on") {
        bad("only blocked.on is expandable.");
      }
      type = "node";
    } else if (type === "leaf") {
      bad(`${segments[i - 1]} holds no objects to expand.`);
    } else if (NODE_REFS.includes(s)) {
      type = "node";
    } else if (s === "children" || s === "descendants") {
      if (inList) {
        throw expandError(
          "expand_not_allowed",
          `${s} can't be expanded on the items of a list: list them with ${s === "children" ? "parent_id" : "ancestor_id"} instead.`,
        );
      }
      if (s === "descendants" && (i > 0 || start !== "node")) {
        throw expandError(
          "expand_not_allowed",
          "descendants can only be expanded on the node retrieved.",
        );
      }
      type = s === "descendants" ? "leaf" : "node";
    } else if (NODE_LEAVES.includes(s)) {
      type = "leaf";
    } else if (s === "blocked") {
      type = "blocked";
    } else {
      bad(
        `a node's ${s} isn't expandable. These are: ${[...NODE_REFS, "children", "descendants", ...NODE_LEAVES, "blocked.on"].join(", ")}.`,
      );
    }
  }
  if (type === "blocked") {
    throw expandError(
      "expand_invalid",
      `Can't expand ${path}: expand blocked.on.`,
    );
  }
}

/** What a node's rendered with. */
export type RenderAt = {
  tree: Tree;
  graphId: string;
  asOf: string | null;
  /** The API's path to the graph: `/api/v1/graphs/gph_…`. */
  base: string;
  /** `descendants_max_depth`. */
  maxDepth?: number;
};

/** A node, and what `trie` asks to expand on it. */
export function renderNode(
  at: RenderAt,
  ref: string,
  trie: Trie,
): ApiNode & Record<string, unknown> {
  const {tree} = at;
  const out: ApiNode & Record<string, unknown> = apiNode(
    tree,
    ref,
    at.graphId,
    at.asOf,
  );
  if (!trie.size) {
    return out;
  }
  const n = tree.node(ref);
  const one = (field: string, other: string | null) => {
    const sub = trie.get(field);
    if (sub && other && tree.has(other)) {
      out[field] = renderNode(at, other, sub);
    }
  };
  one("parent", tree.parentOf(ref));
  one("root", tree.rootOf(ref));
  one("session", tree.sessionOf(ref));
  one("requested_by", n.requested_by ?? null);
  const blockedOn = trie.get("blocked")?.get("on");
  if (blockedOn && out.blocked && n.blocked) {
    out.blocked = {
      ...out.blocked,
      on: n.blocked.on
        .filter(r => tree.has(r))
        .map(r => renderNode(at, r, blockedOn)),
    } as ApiNode["blocked"];
  }
  const children = trie.get("children");
  if (children) {
    const refs = tree.childrenOf(ref);
    out.children = list(
      at,
      {parent_id: nodeId(ref), order: "tree"},
      refs.slice(0, CHILDREN_SHOWN).map(c => renderNode(at, c, children)),
      refs.length > CHILDREN_SHOWN,
    );
  }
  const descendants = trie.get("descendants");
  if (descendants) {
    const refs = tree.under(ref, at.maxDepth ?? Infinity);
    out.descendants = list(
      at,
      {
        ancestor_id: nodeId(ref),
        order: "tree",
        ...(at.maxDepth !== undefined ? {max_depth: String(at.maxDepth)} : {}),
      },
      refs.slice(0, DESCENDANTS_SHOWN).map(d => renderNode(at, d, descendants)),
      refs.length > DESCENDANTS_SHOWN,
    );
  }
  if (trie.has("tasks")) {
    out.tasks = apiTasks(n);
  }
  if (trie.has("spawns")) {
    out.spawns = apiSpawns(n);
  }
  if (trie.has("waits")) {
    out.waits = apiWaits(n);
  }
  if (trie.has("messages")) {
    out.messages = apiMessages(tree, n);
  }
  return out;
}

/** An embedded list: its first items, and a `url` (with `as_of`) for the rest. */
function list(
  at: RenderAt,
  query: Record<string, string>,
  data: Array<unknown>,
  hasMore: boolean,
) {
  const params = new URLSearchParams(query);
  if (at.asOf) {
    params.set("as_of", at.asOf);
  }
  return {
    object: "list" as const,
    url: `${at.base}/nodes?${params}`,
    as_of_event_id: at.asOf,
    has_more: hasMore,
    data,
  };
}
