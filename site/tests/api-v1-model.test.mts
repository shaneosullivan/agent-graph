// Unit tests for the graph API's model of a graph (lib/api/model.ts), and
// what each event changed (lib/api/changes.ts), on the example logs, run
// through the site's own WebAssembly reducer:  npm run test:unit

import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {register} from "node:module";
import {test} from "node:test";

register("../scripts/resolve-ts.mjs", import.meta.url);
const {LogReducer} = await import("../lib/api/reducer.ts");
const {Tree, apiNode, counts, apiTasks, apiSpawns, apiWaits, apiMessages} =
  await import("../lib/api/model.ts");
const {applySteps, same} = await import("../lib/api/changes.ts");
const {nodeId, parseNodeId} = await import("../lib/api/ids.ts");
const {examples, trimmed, line} = await import("./api-v1-fixtures.mts");

const spec = JSON.parse(
  readFileSync(new URL("../openapi.json", import.meta.url), "utf8"),
);
const required = (schema: string) =>
  [...spec.components.schemas[schema].required].sort();

async function load(text: string) {
  const r = await LogReducer.create();
  r.append(text);
  return r;
}

const EXAMPLES = examples();

test("where each node sits: parent, root, session, depth, what's under it", async () => {
  const r = await load(EXAMPLES);
  const tree = new Tree(r.state().nodes);
  const refs = Object.keys(tree.nodes);
  assert.ok(refs.length > 20, "the examples have plenty");
  const order = tree.treeOrder();
  assert.deepEqual([...order].sort(), [...refs].sort(), "each node once");
  const place = new Map(order.map((ref, i) => [ref, i]));
  for (const ref of refs) {
    const parent = tree.parentOf(ref);
    const ancestors = tree.ancestorsOf(ref);
    assert.equal(tree.depthOf(ref), ancestors.length);
    assert.equal(tree.rootOf(ref), ancestors.at(-1) ?? ref);
    if (parent) {
      assert.ok(place.get(parent)! < place.get(ref)!, "a parent comes first");
      assert.ok(tree.childrenOf(parent).includes(ref));
    }
    const session = tree.sessionOf(ref);
    assert.ok(session === ref || ancestors.includes(session));
    assert.ok(
      tree.node(session).kind === "session" || session === tree.rootOf(ref),
    );
    // Its descendants are exactly the nodes with it among their ancestors.
    const under = refs.filter(o => tree.ancestorsOf(o).includes(ref));
    assert.deepEqual([...tree.under(ref)].sort(), under.sort());
    assert.equal(tree.descendantCount(ref), under.length);
    // A level at a time.
    assert.deepEqual(tree.under(ref, 1), tree.childrenOf(ref));
  }
  // Roots, oldest first, and an agent always in some session.
  const roots = tree.roots();
  const created = roots.map(ref =>
    Date.parse(tree.node(ref).started_at ?? tree.node(ref).last_event_at),
  );
  assert.deepEqual(
    created,
    [...created].sort((a, b) => a - b),
  );
});

test("a node as the API shows it: every field the spec requires, named as it names them", async () => {
  const r = await load(EXAMPLES);
  const tree = new Tree(r.state().nodes);
  const want = required("Node");
  for (const ref of Object.keys(tree.nodes)) {
    const n = apiNode(tree, ref, "gph_AbCdEf123456", "evt_X");
    assert.deepEqual(Object.keys(n).sort(), want, ref);
    assert.equal(n.id, nodeId(ref));
    assert.equal(parseNodeId(n.id), ref);
    assert.equal(n.provider_ref, ref);
    assert.equal(n.object, "node");
    assert.equal(n.graph_id, "gph_AbCdEf123456");
    assert.equal(n.as_of_event_id, "evt_X");
    // Ids, never objects, and each an id there is.
    for (const id of [n.root_id, n.session_id]) {
      assert.ok(tree.has(parseNodeId(id)!), `${ref}: ${id}`);
    }
    assert.equal(n.parent_id === null, tree.parentOf(ref) === null);
    assert.equal(
      n.task_counts.open,
      n.task_counts.pending + n.task_counts.in_progress,
    );
    assert.equal(
      n.task_counts.total,
      n.task_counts.pending +
        n.task_counts.in_progress +
        n.task_counts.completed,
    );
    if (n.blocked) {
      assert.deepEqual(
        Object.keys(n.blocked).sort(),
        required("Blocked")
          .filter(k => k !== "on")
          .sort(),
      );
      assert.ok(!["completed", "failed", "canceled"].includes(n.state));
    }
    assert.equal(typeof n.created, "number");
    assert.equal(typeof n.last_event, "number");
    const raw = tree.node(ref);
    for (const t of apiTasks(raw)) {
      assert.deepEqual(Object.keys(t).sort(), required("Task"));
    }
    for (const s of apiSpawns(raw)) {
      assert.deepEqual(Object.keys(s).sort(), required("Spawn"));
    }
    for (const w of apiWaits(raw)) {
      assert.deepEqual(Object.keys(w).sort(), required("Wait"));
      assert.ok(w.kind === "spawn" || w.kind === "explicit");
    }
    for (const m of apiMessages(tree, raw)) {
      assert.deepEqual(Object.keys(m).sort(), required("Message"));
      // The other side: a node here, by id, or the tool's name for it.
      assert.ok((m.peer_id === null) !== (m.peer_name === null));
    }
  }
  const c = counts(tree);
  assert.deepEqual(Object.keys(c).sort(), required("GraphCounts"));
  assert.equal(c.nodes, c.sessions + c.agents);
  assert.equal(
    Object.values(c.by_state).reduce((a, b) => a + b, 0),
    c.nodes,
  );
});

/**
 * Each event's changes, applied in turn to the API's nodes before it, give
 * the API's nodes after it: every field but those that change with time.
 */
async function changesAddUp(text: string, from = 0) {
  const r = await load(text);
  const {events} = r.events();
  const graph = "gph_AbCdEf123456";
  const {before, events: steps} = r.changes(from, events.length - 1);
  const applied = applySteps(
    before,
    steps,
    events.slice(from).map(e => e.node),
    graph,
  );
  // What changes leave out: what never changes, and what changes with time.
  const strip = (n: Record<string, unknown>) => {
    const {
      stale: _s,
      as_of_event_id: _a,
      id: _i,
      object: _o,
      graph_id: _g,
      provider_ref: _p,
      ...rest
    } = n;
    return rest;
  };
  // The API's nodes before `from`.
  const beforeTree = new Tree(before);
  const nodes = new Map<string, Record<string, unknown>>(
    Object.keys(before).map(ref => [
      nodeId(ref),
      strip(apiNode(beforeTree, ref, graph, null)),
    ]),
  );
  let changed = 0;
  applied.forEach((a, k) => {
    const i = from + k;
    for (const c of a.changes) {
      const was = nodes.get(c.node_id);
      assert.equal(c.created, !was, `${events[i].id}: ${c.node_id} created?`);
      const now = {...(was ?? {})};
      for (const [field, {from: f, to}] of Object.entries(c.fields)) {
        assert.ok(
          same(now[field] ?? null, f),
          `${events[i].id}: ${field} was ${JSON.stringify(f)}`,
        );
        now[field] = to;
        changed += 1;
      }
      nodes.set(c.node_id, now);
    }
    // The event's own node comes first.
    if (a.changes.length) {
      assert.equal(a.changes[0].node_id, nodeId(events[i].node));
    }
    const tree = new Tree(r.state(events[i].id).nodes);
    for (const ref of Object.keys(tree.nodes)) {
      const want = strip(apiNode(tree, ref, graph, null));
      const got = nodes.get(nodeId(ref));
      assert.ok(got, `${events[i].id}: ${ref} is there`);
      for (const [field, value] of Object.entries(want)) {
        // (A field a created node didn't list is one it had empty.)
        assert.ok(
          same(got[field] ?? null, value),
          `${events[i].id}: ${ref}'s ${field}: ${JSON.stringify(got[field])} isn't ${JSON.stringify(value)}`,
        );
      }
    }
    if (a.node) {
      assert.deepEqual(
        strip(a.node),
        strip(apiNode(tree, events[i].node, graph, null)),
      );
    }
  });
  return changed;
}

test("each event's changes add up to the graph after it, from a log's start", async () => {
  const changed = await changesAddUp(EXAMPLES);
  assert.ok(changed > 200, `only ${changed} fields changed`);
});

test("…from partway through", async () => {
  const r = await load(EXAMPLES);
  await changesAddUp(EXAMPLES, Math.floor(r.events().events.length / 2));
});

test("…and from a keyframe, as a live share's log starts", async () => {
  const cut = trimmed(EXAMPLES, 40);
  assert.match(cut.split("\n")[0], /"type":"keyframe"/);
  const r = await load(cut);
  assert.equal(r.events().base, true);
  assert.ok(r.events().events.length < 130);
  await changesAddUp(cut);
});

test("a node started under another changes the counts of everything above it", async () => {
  const text =
    line(0, "x:s", "session.started") +
    line(
      1,
      "x:s/a",
      "agent.spawned",
      {agent_type: "Explore"},
      {parent: "x:s"},
    ) +
    line(2, "x:s/b", "agent.spawned", {}, {parent: "x:s/a"}) +
    line(3, "x:s/b", "agent.finished", {status: "completed"});
  const r = await load(text);
  const {events} = r.events();
  const {before, events: steps} = r.changes(0, events.length - 1);
  const applied = applySteps(
    before,
    steps,
    events.map(e => e.node),
    "gph_x",
  );
  const fields = (k: number, ref: string) =>
    applied[k].changes.find(c => c.node_id === nodeId(ref))?.fields ?? {};

  assert.equal(applied[0].changes.length, 1);
  assert.equal(applied[0].changes[0].created, true);
  assert.deepEqual(fields(0, "x:s").kind, {from: null, to: "session"});

  // An agent under the session: made, and the session's counts go up.
  assert.equal(
    applied[1].changes[0].node_id,
    nodeId("x:s/a"),
    "its own node first",
  );
  assert.equal(applied[1].changes[0].created, true);
  assert.deepEqual(fields(1, "x:s/a").parent_id, {
    from: null,
    to: nodeId("x:s"),
  });
  assert.deepEqual(fields(1, "x:s/a").depth, {from: null, to: 1});
  assert.deepEqual(fields(1, "x:s").child_count, {from: 0, to: 1});
  assert.deepEqual(fields(1, "x:s").descendant_count, {from: 0, to: 1});

  // One under that: its parent's and its grandparent's counts, but only
  // the grandparent's descendants.
  assert.deepEqual(fields(2, "x:s/b").root_id, {from: null, to: nodeId("x:s")});
  assert.deepEqual(fields(2, "x:s/b").depth, {from: null, to: 2});
  assert.deepEqual(fields(2, "x:s/a").child_count, {from: 0, to: 1});
  assert.deepEqual(fields(2, "x:s").descendant_count, {from: 1, to: 2});
  assert.equal(fields(2, "x:s").child_count, undefined);

  // Finishing changes the node itself.
  assert.deepEqual(fields(3, "x:s/b").state, {
    from: "working",
    to: "completed",
  });
  assert.equal(applied[3].node?.state, "completed");
});

test("comparing values", () => {
  assert.ok(same({a: [1, {b: null}]}, {a: [1, {b: null}]}));
  assert.ok(!same({a: 1}, {a: 1, b: 2}));
  assert.ok(!same([1], {0: 1}));
  assert.ok(!same(null, {}));
  assert.ok(same(null, null));
  assert.ok(!same(1, "1"));
});
