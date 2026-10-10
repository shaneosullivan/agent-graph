// The graph as the API shows it (site/openapi.json, "Node"): the reducer's
// nodes (src/reducer.rs, by way of the WebAssembly), with where each sits
// in its tree worked out, and named as the API names things: a field that
// refers to another object ends `_id`, and is always there.

import {nodeId} from "./ids";

/** A node as the reducer gives it (src/reducer.rs `Node`). */
export type RNode = {
  id: string;
  kind: "session" | "agent";
  provider: string;
  parent?: string;
  children: Array<string>;
  state: string;
  summary?: string;
  attention?: string;
  title?: string;
  cwd?: string;
  agent_type?: string;
  purpose?: string;
  background?: boolean;
  spawned_by?: string;
  requested_by?: string;
  link?: string;
  tasks: Array<{
    id: string;
    text: string;
    active_text?: string;
    status: string;
  }>;
  spawns: Array<{
    call_id: string;
    kind: string;
    agent_type?: string;
    purpose?: string;
    background: boolean;
    run?: boolean;
    title?: string;
    child?: string;
    returned: boolean;
    requested_at: string;
  }>;
  waits: Array<{
    wait_id: string;
    on?: string;
    reason?: string;
    open: boolean;
    spawn: boolean;
    started_at: string;
    ended_at?: string;
  }>;
  messages: Array<{
    message_id: string;
    direction: string;
    peer: string;
    reply_to?: string;
    summary?: string;
    body?: string;
    ts: string;
  }>;
  started_at?: string;
  ended_at?: string;
  last_event_at: string;
  headline?: string;
  open_tasks: number;
  blocked?: {
    on: Array<string>;
    starting: number;
    nodes: number;
    open_tasks: number;
    cycle: boolean;
  };
  stale: boolean;
  background_agents?: number;
};

export type RNodes = Record<string, RNode>;

export const STATES = [
  "working",
  "input_required",
  "idle",
  "completed",
  "failed",
  "canceled",
] as const;

const FINISHED = new Set(["completed", "failed", "canceled"]);

/** An RFC 3339 time as Unix milliseconds; null if there isn't one. */
export function ms(time: string | undefined): number | null {
  if (!time) {
    return null;
  }
  const t = Date.parse(time);
  return Number.isNaN(t) ? null : t;
}

/**
 * The nodes at one moment, and where each sits: its parent (one in the
 * graph: a node whose parent isn't is a root), its root, its session, its
 * depth, and what's under it. Worked out as asked for, and kept.
 */
export class Tree {
  private readonly rootMemo = new Map<string, string>();
  private readonly depthMemo = new Map<string, number>();
  private readonly sizeMemo = new Map<string, number>();
  private ordered: Array<string> | null = null;
  readonly nodes: RNodes;

  constructor(nodes: RNodes) {
    this.nodes = nodes;
  }

  has(ref: string): boolean {
    return Object.hasOwn(this.nodes, ref);
  }

  node(ref: string): RNode {
    return this.nodes[ref];
  }

  /** Its parent, if that's in the graph. */
  parentOf(ref: string): string | null {
    const parent = this.nodes[ref]?.parent;
    return parent && parent !== ref && this.has(parent) ? parent : null;
  }

  /** Its parents, nearest first. */
  ancestorsOf(ref: string): Array<string> {
    const out: Array<string> = [];
    const seen = new Set([ref]);
    for (let p = this.parentOf(ref); p && !seen.has(p); p = this.parentOf(p)) {
      seen.add(p);
      out.push(p);
    }
    return out;
  }

  rootOf(ref: string): string {
    let root = this.rootMemo.get(ref);
    if (root === undefined) {
      root = this.ancestorsOf(ref).at(-1) ?? ref;
      this.rootMemo.set(ref, root);
    }
    return root;
  }

  depthOf(ref: string): number {
    let depth = this.depthMemo.get(ref);
    if (depth === undefined) {
      depth = this.ancestorsOf(ref).length;
      this.depthMemo.set(ref, depth);
    }
    return depth;
  }

  /** The session it runs in: itself, or the nearest session above it (else its root). */
  sessionOf(ref: string): string {
    if (this.nodes[ref]?.kind === "session") {
      return ref;
    }
    return (
      this.ancestorsOf(ref).find(a => this.nodes[a].kind === "session") ??
      this.rootOf(ref)
    );
  }

  /** The nodes directly under it, oldest first (as the reducer keeps them). */
  childrenOf(ref: string): Array<string> {
    return (this.nodes[ref]?.children ?? []).filter(
      c => c !== ref && this.parentOf(c) === ref,
    );
  }

  /** How many nodes are under it, at any depth. */
  descendantCount(ref: string): number {
    let size = this.sizeMemo.get(ref);
    if (size === undefined) {
      size = this.under(ref).length;
      this.sizeMemo.set(ref, size);
    }
    return size;
  }

  /**
   * The nodes under `ref`, in tree order (each parent before its children,
   * siblings oldest first), at most `maxDepth` levels below it.
   */
  under(ref: string, maxDepth = Infinity): Array<string> {
    const out: Array<string> = [];
    const seen = new Set([ref]);
    const walk = (at: string, depth: number) => {
      if (depth > maxDepth) {
        return;
      }
      for (const child of this.childrenOf(at)) {
        if (!seen.has(child)) {
          seen.add(child);
          out.push(child);
          walk(child, depth + 1);
        }
      }
    };
    walk(ref, 1);
    return out;
  }

  /** The roots, oldest first. */
  roots(): Array<string> {
    return Object.keys(this.nodes)
      .filter(ref => this.parentOf(ref) === null)
      .sort((a, b) => byCreated(this.nodes[a], this.nodes[b]));
  }

  /** Every node, in tree order: each root, oldest first, then what's under it. */
  treeOrder(): Array<string> {
    if (!this.ordered) {
      const out: Array<string> = [];
      for (const root of this.roots()) {
        out.push(root, ...this.under(root));
      }
      // (A loop of parents has no root: its nodes go last.)
      const placed = new Set(out);
      for (const ref of Object.keys(this.nodes).sort()) {
        if (!placed.has(ref)) {
          out.push(ref);
        }
      }
      this.ordered = out;
    }
    return this.ordered;
  }
}

/** When it started (or was first seen). */
export function createdOf(node: RNode): number {
  return ms(node.started_at) ?? ms(node.last_event_at) ?? 0;
}

function byCreated(a: RNode, b: RNode): number {
  return (
    createdOf(a) - createdOf(b) || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0)
  );
}

/** Newest first: most recently started, then by id. */
export function byNewest(a: RNode, b: RNode): number {
  return (
    createdOf(b) - createdOf(a) || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0)
  );
}

/** What a node shows, as the API names it: everything but what it's expanded with. */
export type ApiNode = {
  id: string;
  object: "node";
  as_of_event_id: string | null;
  graph_id: string;
  provider_ref: string;
  provider: string;
  kind: "session" | "agent";
  parent_id: string | null;
  root_id: string;
  session_id: string;
  requested_by_id: string | null;
  depth: number;
  state: string;
  stale: boolean;
  attention: string | null;
  title: string | null;
  summary: string | null;
  headline: string | null;
  purpose: string | null;
  agent_type: string | null;
  background: boolean | null;
  link_method: string | null;
  spawn_call_id: string | null;
  cwd: string | null;
  created: number;
  ended: number | null;
  last_event: number;
  child_count: number;
  descendant_count: number;
  background_agent_count: number;
  task_counts: {
    total: number;
    pending: number;
    in_progress: number;
    completed: number;
    open: number;
  };
  blocked: {
    on_ids: Array<string>;
    starting: number;
    node_count: number;
    open_tasks: number;
    cycle: boolean;
  } | null;
};

/** Node `ref` of `tree`, as the API shows it. */
export function apiNode(
  tree: Tree,
  ref: string,
  graphId: string,
  asOf: string | null,
): ApiNode {
  const n = tree.node(ref);
  const parent = tree.parentOf(ref);
  const count = (status: string) =>
    n.tasks.filter(t => t.status === status).length;
  const pending = count("pending");
  const inProgress = count("in_progress");
  return {
    id: nodeId(ref),
    object: "node",
    as_of_event_id: asOf,
    graph_id: graphId,
    provider_ref: ref,
    provider: n.provider,
    kind: n.kind,
    parent_id: parent ? nodeId(parent) : null,
    root_id: nodeId(tree.rootOf(ref)),
    session_id: nodeId(tree.sessionOf(ref)),
    requested_by_id: n.requested_by ? nodeId(n.requested_by) : null,
    depth: tree.depthOf(ref),
    state: n.state,
    stale: n.stale,
    attention: n.attention ?? null,
    title: n.title ?? null,
    summary: n.summary ?? null,
    headline: n.headline ?? null,
    purpose: n.purpose ?? null,
    agent_type: n.agent_type ?? null,
    background: n.background ?? null,
    link_method: n.link ?? null,
    spawn_call_id: n.spawned_by ?? null,
    cwd: n.cwd ?? null,
    created: createdOf(n),
    ended: ms(n.ended_at),
    last_event: ms(n.last_event_at) ?? 0,
    child_count: tree.childrenOf(ref).length,
    descendant_count: tree.descendantCount(ref),
    background_agent_count: n.background_agents ?? 0,
    task_counts: {
      total: n.tasks.length,
      pending,
      in_progress: inProgress,
      completed: count("completed"),
      open: pending + inProgress,
    },
    blocked:
      n.blocked && !FINISHED.has(n.state)
        ? {
            on_ids: n.blocked.on.map(nodeId),
            starting: n.blocked.starting,
            node_count: n.blocked.nodes,
            open_tasks: n.blocked.open_tasks,
            cycle: n.blocked.cycle,
          }
        : null,
  };
}

/** Its task list, as the API shows it (`expand[]=tasks`). */
export function apiTasks(n: RNode) {
  return n.tasks.map(t => ({
    id: t.id,
    text: t.text,
    active_text: t.active_text ?? null,
    status: t.status,
  }));
}

/** The children it has asked for (`expand[]=spawns`). */
export function apiSpawns(n: RNode) {
  return n.spawns.map(s => ({
    call_id: s.call_id,
    kind: s.kind,
    agent_type: s.agent_type ?? null,
    purpose: s.purpose ?? null,
    background: s.background,
    run: s.run ?? null,
    title: s.title ?? null,
    child_id: s.child ? nodeId(s.child) : null,
    returned: s.returned,
    requested: ms(s.requested_at) ?? 0,
  }));
}

/** What it has waited on (`expand[]=waits`). */
export function apiWaits(n: RNode) {
  return n.waits.map(w => ({
    id: w.wait_id,
    kind: w.spawn ? "spawn" : "explicit",
    on_id: w.on ? nodeId(w.on) : null,
    reason: w.reason ?? null,
    open: w.open,
    started: ms(w.started_at) ?? 0,
    ended: ms(w.ended_at),
  }));
}

/** Its messages (`expand[]=messages`): the other side by its id when it's a node here. */
export function apiMessages(tree: Tree, n: RNode) {
  return n.messages.map(m => ({
    id: m.message_id,
    direction: m.direction,
    peer_id: tree.has(m.peer) ? nodeId(m.peer) : null,
    peer_name: tree.has(m.peer) ? null : m.peer,
    reply_to_message_id: m.reply_to ?? null,
    summary: m.summary ?? null,
    body: m.body ?? null,
    created: ms(m.ts) ?? 0,
  }));
}

/** How a graph's nodes stand (a graph's `counts`). */
export function counts(tree: Tree) {
  const nodes = Object.values(tree.nodes);
  const byState = Object.fromEntries(STATES.map(s => [s, 0])) as Record<
    (typeof STATES)[number],
    number
  >;
  for (const n of nodes) {
    if (n.state in byState) {
      byState[n.state as (typeof STATES)[number]] += 1;
    }
  }
  return {
    nodes: nodes.length,
    sessions: nodes.filter(n => n.kind === "session").length,
    agents: nodes.filter(n => n.kind === "agent").length,
    stale: nodes.filter(n => n.stale).length,
    blocked: nodes.filter(n => n.blocked && !FINISHED.has(n.state)).length,
    by_state: byState,
  };
}
