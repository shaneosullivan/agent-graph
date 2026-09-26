// site-source.js, the page's data source on the site, in jsdom: it reads a
// log's chunks from the API and hands them to the site's WebAssembly.
// npm run test:unit

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { deflateRawSync } from "node:zlib";
import { JSDOM } from "jsdom";

const source = readFileSync(new URL("../public/viewer/site-source.js", import.meta.url), "utf8");
const wasm = readFileSync(new URL("../public/viewer/agent_graph.wasm", import.meta.url));

const line = (n, node, type, data = {}) =>
  JSON.stringify({ v: 1, id: `01K${String(n).padStart(23, "0")}`, ts: `2026-09-25T10:00:0${n}.000Z`, type, node, data }) +
  "\n";
const key = (offset) => String(offset).padStart(15, "0");

/**
 * A keyframe (src/reducer.rs), `n` seconds in, whose state is session
 * `node` (with a long title, if `title` is given).
 */
const keyframe = (n, node, title) => {
  const state = {
    nodes: {
      [node]: {
        id: node,
        kind: "session",
        ...(title ? { title } : {}),
        provider: "x",
        children: [],
        state: "idle",
        tasks: [],
        spawns: [],
        waits: [],
        messages: [],
        last_event_at: `2026-09-25T10:00:0${n}.000Z`,
        open_tasks: 0,
        stale: false,
      },
    },
    processes: {},
    waiting_on: {},
    requesters: {},
    ended: {},
    started: {},
  };
  // (Stored, not deflated, so a long title makes a long line.)
  const text = deflateRawSync(Buffer.from(JSON.stringify(state)), { level: 0 }).toString("base64");
  return (
    JSON.stringify({
      v: 1,
      id: `01K${String(n).padStart(23, "0")}~000000`,
      ts: `2026-09-25T10:00:0${n}.000Z`,
      type: "keyframe",
      node: "agent-graph:keyframe",
      data: { part: 0, parts: 1, text },
    }) + "\n"
  );
};
const bytes = (text) => Buffer.byteLength(text);

/**
 * Loads site-source.js for a live log whose content reads are answered, in
 * turn, by `reads` (`{ text, first }`; each is served once, then empty
 * reads). Timers run a thousand times faster. Returns the window and the
 * `after` of every content read made.
 */
function load(t, reads) {
  const dom = new JSDOM(
    `<script id="agent-graph-config" type="application/json">{"id":"abc","live":true}</script><div id="view"></div>`,
    { url: "https://example.test/l/abc", runScripts: "outside-only" },
  );
  const { window } = dom;
  t.after(() => window.close());
  const afters = [];
  const queue = [...reads];
  window.fetch = async (url) => {
    if (url === "/viewer/agent_graph.wasm") return { arrayBuffer: async () => wasm };
    const after = new URL(url, "https://example.test").searchParams.get("after");
    afters.push(after);
    const read = queue.shift() ?? { text: "" };
    const headers = new Map([
      ...(read.first !== undefined ? [["x-first-chunk", key(read.first)]] : []),
      ...(read.text ? [["x-last-chunk", key(read.first + bytes(read.text) - 1)]] : []),
    ]);
    // A real one: its text() drops a byte-order mark at its start, as a
    // browser's does.
    return new Response(read.text, { status: 200, headers: Object.fromEntries(headers) });
  };
  const setTimeout = window.setTimeout.bind(window);
  window.setTimeout = (fn, ms) => setTimeout(fn, ms / 1000);
  window.eval(source);
  return { window, afters };
}

async function until(check, ms = 3000) {
  const start = Date.now();
  for (;;) {
    const value = await check();
    if (value) return value;
    if (Date.now() - start > ms) throw new Error(`timed out waiting for ${check}`);
    await new Promise((r) => setTimeout(r, 5));
  }
}

// Keyframes: a live share keeps only its last two keyframes' worth. A read
// that doesn't start where the last one ended, with a keyframe, means the
// log's start was trimmed while it was read (perhaps only partway so far):
// what the page holds is broken, so it starts again from that keyframe,
// whatever's still before it.
test("a trim in what's read starts the log again at its keyframe", async (t) => {
  const first = line(1, "x:a", "session.started") + line(2, "x:a", "status", { state: "working" });
  const later = keyframe(3, "x:k") + line(4, "x:b", "session.started");
  const { window, afters } = load(t, [
    { text: first, first: 0 },
    // Not where the last read ended: the start was trimmed.
    { text: later, first: 500 },
  ]);
  const src = await until(() => window.agentGraphSource);
  const changed = new Promise((resolve) => src.subscribe(resolve, () => {}));
  assert.deepEqual(Object.keys((await src.graph()).sessions), ["x:a"]);
  await changed;
  await until(() => afters.length >= 3);
  assert.deepEqual(afters.slice(0, 3), ["", key(bytes(first) - 1), key(500 + bytes(later) - 1)]);
  assert.deepEqual(Object.keys((await src.graph()).sessions).sort(), ["x:b", "x:k"], "only what's there now");
});

// A gap that isn't a trim's (a log written with one) is read past.
test("a keyframe line longer than a read's first few KB is still one", async (t) => {
  const first = line(1, "x:a", "session.started");
  const long = keyframe(2, "x:k", "t".repeat(200_000));
  assert.ok(long.length > 150_000);
  const { window, afters } = load(t, [
    { text: first, first: 0 },
    { text: long + line(3, "x:b", "session.started"), first: 500 },
  ]);
  const src = await until(() => window.agentGraphSource);
  const changed = new Promise((resolve) => src.subscribe(resolve, () => {}));
  await changed;
  await until(() => afters.length >= 3);
  assert.deepEqual(Object.keys((await src.graph()).sessions).sort(), ["x:b", "x:k"]);
});

test("a gap without a keyframe after it is read past", async (t) => {
  const first = line(1, "x:a", "session.started");
  const later = line(2, "x:b", "session.started");
  const { window, afters } = load(t, [
    { text: first, first: 0 },
    { text: later, first: 500 },
  ]);
  const src = await until(() => window.agentGraphSource);
  const changed = new Promise((resolve) => src.subscribe(resolve, () => {}));
  await changed;
  await until(() => afters.length >= 3);
  assert.deepEqual(Object.keys((await src.graph()).sessions).sort(), ["x:a", "x:b"]);
});

// A read that starts with a byte-order mark (the first line of a file
// Windows PowerShell wrote) is counted as it was stored, mark and all, so
// the next isn't taken for a trim, even when it starts with a keyframe.
test("a read's bytes are counted, a byte-order mark and all", async (t) => {
  const first = "\uFEFFnot an event\n" + line(1, "x:a", "session.started");
  const later = keyframe(2, "x:k") + line(3, "x:b", "session.started");
  const { window, afters } = load(t, [
    { text: first, first: 0 },
    { text: later, first: bytes(first) },
  ]);
  const src = await until(() => window.agentGraphSource);
  const changed = new Promise((resolve) => src.subscribe(resolve, () => {}));
  await changed;
  await until(() => afters.length >= 3);
  assert.deepEqual(Object.keys((await src.graph()).sessions).sort(), ["x:a", "x:b"], "all of it, from the start");
});

test("reads that follow on are kept", async (t) => {
  const first = line(1, "x:a", "session.started");
  const later = line(2, "x:b", "session.started");
  const { window, afters } = load(t, [
    { text: first, first: 0 },
    { text: later, first: bytes(first) },
  ]);
  const src = await until(() => window.agentGraphSource);
  const changed = new Promise((resolve) => src.subscribe(resolve, () => {}));
  await changed;
  assert.deepEqual(Object.keys((await src.graph()).sessions).sort(), ["x:a", "x:b"]);
  assert.equal(afters[1], key(bytes(first) - 1));
});
