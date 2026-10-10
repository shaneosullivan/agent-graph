// What each event changed (site/openapi.json, "Event" `changes`): every
// node it changed, field by field, as the API shows nodes. The reducer
// says which of its nodes each event changed (wasm `changes`); what that
// does to where nodes sit (a parent's `child_count`, every ancestor's
// `descendant_count`, the `root_id` and `depth` of a subtree that moved) is
// worked out here.

import {nodeId} from "./ids";
import {type ApiNode, apiNode, type RNode, type RNodes, Tree} from "./model";

/** How one event changed one node. */
export type NodeChange = {
  node_id: string;
  created: boolean;
  fields: Record<string, {from: unknown; to: unknown}>;
};

/** What a node's change leaves out: what never changes, and what changes with time, not events. */
const UNCOMPARED = new Set([
  "id",
  "object",
  "as_of_event_id",
  "graph_id",
  "provider_ref",
  "stale",
]);

/** One event's step, as the reducer gives it. */
export type Step = {
  id: string;
  changed: Array<RNode>;
  removed: Array<string>;
};

/** What one event did: its changes, and its node just after it (null if there's none). */
export type Applied = {changes: Array<NodeChange>; node: ApiNode | null};

/**
 * Applies each of `steps` in turn to `before` (the reducer's nodes before
 * the first), returning what each changed, starting with the node it's
 * about (`subjects`, by step), and that node just after it.
 */
export function applySteps(
  before: RNodes,
  steps: ReadonlyArray<Step>,
  subjects: ReadonlyArray<string>,
  graphId: string,
): Array<Applied> {
  const nodes: RNodes = {...before};
  const out: Array<Applied> = [];
  steps.forEach((step, i) => {
    const prev = new Tree(nodes);
    const touched = affected(prev, step);
    const subject = subjects[i];
    touched.add(subject);
    const was = new Map<string, ApiNode | null>();
    for (const ref of touched) {
      was.set(ref, prev.has(ref) ? apiNode(prev, ref, graphId, null) : null);
    }
    for (const node of step.changed) {
      nodes[node.id] = node;
    }
    for (const ref of step.removed) {
      delete nodes[ref];
    }
    const next = new Tree(nodes);
    const changes: Array<NodeChange> = [];
    let after: ApiNode | null = null;
    for (const ref of [...touched].sort(subjectFirst(subject))) {
      const now = next.has(ref) ? apiNode(next, ref, graphId, null) : null;
      if (ref === subject) {
        after = now;
      }
      const change = diff(ref, was.get(ref) ?? null, now);
      if (change) {
        changes.push(change);
      }
    }
    out.push({changes, node: after});
  });
  return out;
}

const subjectFirst = (subject: string) => (a: string, b: string) =>
  a === subject ? -1 : b === subject ? 1 : a < b ? -1 : a > b ? 1 : 0;

/**
 * The nodes whose API fields a step may change: those it changed or
 * removed, everything above them (before and after: counts of what's
 * under them), and, for one created, removed or moved, everything under
 * it (its root and depth).
 */
function affected(prev: Tree, step: Step): Set<string> {
  const out = new Set<string>();
  const changedById = new Map(step.changed.map(n => [n.id, n]));
  const parentAfter = (ref: string) => {
    const p = (changedById.get(ref) ?? prev.nodes[ref])?.parent;
    return p && p !== ref ? p : null;
  };
  const above = (ref: string) => {
    for (const a of prev.ancestorsOf(ref)) {
      out.add(a);
    }
    const seen = new Set([ref]);
    for (let p = parentAfter(ref); p && !seen.has(p); p = parentAfter(p)) {
      seen.add(p);
      out.add(p);
    }
  };
  const below = (ref: string) => {
    for (const d of prev.under(ref)) {
      out.add(d);
    }
    for (const c of changedById.get(ref)?.children ?? []) {
      out.add(c);
      for (const d of prev.under(c)) {
        out.add(d);
      }
    }
  };
  for (const node of step.changed) {
    out.add(node.id);
    above(node.id);
    const old = prev.nodes[node.id];
    if (!old || old.parent !== node.parent) {
      below(node.id);
    }
  }
  for (const ref of step.removed) {
    out.add(ref);
    above(ref);
    below(ref);
  }
  // A node whose parent is created (or removed) moves with it.
  const comeOrGone = new Set([
    ...step.changed.filter(n => !prev.has(n.id)).map(n => n.id),
    ...step.removed,
  ]);
  if (comeOrGone.size) {
    for (const ref of Object.keys(prev.nodes)) {
      const p = prev.nodes[ref].parent;
      if (p && comeOrGone.has(p)) {
        out.add(ref);
        below(ref);
      }
    }
  }
  return out;
}

/** How `from` became `to`, field by field; null if nothing changed. */
function diff(
  ref: string,
  from: ApiNode | null,
  to: ApiNode | null,
): NodeChange | null {
  if (!from && !to) {
    return null;
  }
  const fields: NodeChange["fields"] = {};
  const keys = Object.keys((to ?? from)!) as Array<keyof ApiNode>;
  for (const key of keys) {
    if (UNCOMPARED.has(key)) {
      continue;
    }
    const a = from ? from[key] : null;
    const b = to ? to[key] : null;
    // A node made: what it has (not every empty field). Otherwise, what moved.
    if (from ? !same(a, b) : b !== null) {
      fields[key] = {from: a, to: b};
    }
  }
  if (from && !Object.keys(fields).length) {
    return null;
  }
  return {node_id: nodeId(ref), created: !from && Boolean(to), fields};
}

/** Whether two JSON values are the same. */
export function same(a: unknown, b: unknown): boolean {
  if (a === b) {
    return true;
  }
  if (
    a === null ||
    b === null ||
    typeof a !== "object" ||
    typeof b !== "object"
  ) {
    return false;
  }
  if (Array.isArray(a) !== Array.isArray(b)) {
    return false;
  }
  const ka = Object.keys(a);
  const kb = Object.keys(b);
  return (
    ka.length === kb.length &&
    ka.every(k =>
      same(
        (a as Record<string, unknown>)[k],
        (b as Record<string, unknown>)[k],
      ),
    )
  );
}
