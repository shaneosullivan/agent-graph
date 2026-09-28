// The site's committed WebAssembly (public/viewer/agent_graph.wasm), loaded
// the way site-source.js loads it, answers as the page expects:
// npm run test:unit

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const bytes = readFileSync(new URL("../public/viewer/agent_graph.wasm", import.meta.url));
const wasm = (await WebAssembly.instantiate(bytes, {})).instance.exports;
const enc = new TextEncoder();
const dec = new TextDecoder();

function put(text) {
  const data = enc.encode(text);
  const ptr = wasm.alloc(data.length);
  new Uint8Array(wasm.memory.buffer, ptr, data.length).set(data);
  return [ptr, data.length];
}

function call(request) {
  const [ptr, len] = put(JSON.stringify(request));
  const out = wasm.query(ptr, len);
  wasm.dealloc(ptr, len);
  const n = wasm.result_len();
  const text = dec.decode(new Uint8Array(wasm.memory.buffer, out, n));
  wasm.dealloc(out, n);
  return JSON.parse(text);
}

const line = (n, node, type, data = {}) =>
  JSON.stringify({
    v: 1,
    id: `01K${String(n).padStart(23, "0")}`,
    ts: `2026-09-25T10:00:0${n}.000Z`,
    type,
    node,
    data,
  });

const [ptr, len] = put(
  [
    line(1, "x:a", "session.started", { cwd: "/w/a" }),
    line(2, "x:a/b", "agent.spawned", { agent_type: "Explore" }),
    line(3, "x:c", "session.started"),
    line(4, "x:c", "wait.started", { wait_id: "w", on: "x:a" }),
  ].join("\n") + "\n",
);
wasm.append(ptr, len);
wasm.dealloc(ptr, len);

// R23: the reply the page reads: every session summarised, the tree asked
// for, what names the nodes it refers to, and which tree that is.
test("the site's WebAssembly sends one tree, and every session's summary", () => {
  const g = call({ op: "graph", env: "site", root: "x:c" });
  assert.equal(g.status, undefined, JSON.stringify(g));
  assert.equal(g.root, "x:c");
  assert.deepEqual(Object.keys(g.nodes), ["x:c"]);
  assert.deepEqual(Object.keys(g.others), ["x:a"]);
  assert.deepEqual(Object.keys(g.sessions).sort(), ["x:a", "x:c"]);
  assert.equal(g.sessions["x:a"].agents, 1);
  assert.equal(g.sessions["x:a"].cwd, "/w/a");

  const newest = call({ op: "graph", env: "site" });
  assert.equal(newest.root, newest.roots[0], "none asked for: the newest");
});

// R56: one request, reducing the log once, brings a refresh all it needs.
test("the site's WebAssembly sends the graph now with its tree's timeline", () => {
  const g = call({ op: "graph", env: "site", root: "x:a" });
  assert.deepEqual(
    g.stops.map((s) => s.node),
    ["x:a", "x:a/b"],
  );
  assert.equal(call({ op: "graph", env: "site", root: "x:c" }).stops.length, 2);
  const step = call({ op: "graph", env: "site", root: "x:a", until: g.stops[0].id });
  assert.equal(step.stops, undefined, "a step's is the same timeline");
});
