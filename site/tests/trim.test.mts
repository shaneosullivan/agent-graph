// Cutting a pasted or uploaded log to share, its last two keyframes' worth
// (lib/trim-core.ts, lib/trim-worker.ts, lib/trim.ts), with the site's
// WebAssembly, as the page's worker uses it:  npm run test:unit

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { register } from "node:module";
import { test } from "node:test";

register("../scripts/resolve-ts.mjs", import.meta.url);
const { runTrim, eventsShared, KEYFRAME_EVERY } = await import("../lib/trim-core.ts");
const { handle } = await import("../lib/trim-worker.ts");
const { forSite } = await import("../lib/trim.ts");
const { summarize } = await import("../lib/upload.ts");
type TrimExports = import("../lib/trim-core.ts").TrimExports;

const bytes = readFileSync(new URL("../public/viewer/agent_graph.wasm", import.meta.url));
const load = async () => (await WebAssembly.instantiate(bytes, {})).instance.exports as unknown as TrimExports;
const wasm = await load();

const line = (n: number) =>
  JSON.stringify({
    v: 1,
    id: `01K${String(n).padStart(23, "0")}`,
    ts: new Date(Date.UTC(2026, 8, 25, 10) + n * 1000).toISOString(),
    type: "status",
    node: `x:s${Math.floor(n / 100)}`,
    data: { state: "working" },
  }) + "\n";
const log = (n: number) => Array.from({ length: n }, (_, i) => line(i)).join("");

// Keyframes: the site keeps a log's last two keyframes' worth.
test("a pasted log is shared from its last keyframe but one", () => {
  for (const n of [0, 5, KEYFRAME_EVERY, 2 * KEYFRAME_EVERY - 1, 2 * KEYFRAME_EVERY, 3456]) {
    const shared = runTrim(wasm, log(n));
    assert.equal(shared.of, n);
    assert.equal(shared.events, eventsShared(n), `${n} events`);
    const lines = shared.text.split("\n").filter(Boolean);
    const keyframes = lines.filter((l) => JSON.parse(l).type === "keyframe").length;
    assert.equal(lines.length - keyframes, shared.events);
    if (n >= 2 * KEYFRAME_EVERY) assert.equal(JSON.parse(lines[0]).type, "keyframe", "it starts at one");
    else assert.equal(shared.text.startsWith(line(0)), n > 0, "from the start");
    // What's shared counts the same, but for what was left out.
    const summary = summarize(shared.text);
    assert.equal(summary.events, shared.events);
    assert.equal(summary.skipped, 0);
  }
  assert.equal(eventsShared(3456), 1456);
});

test("it says how far along it is, up to done", () => {
  const seen: number[] = [];
  runTrim(wasm, log(30_000), (p) => seen.push(p));
  assert.ok(seen.length > 5, `${seen.length} reports`);
  assert.ok(seen.every((p, i) => p >= 0 && p <= 1 && (i === 0 || p >= seen[i - 1])));
  assert.equal(seen[seen.length - 1], 1);
});

test("the worker says how far along it is, then what to share", async () => {
  const posted: object[] = [];
  await handle(log(2500), (m) => posted.push(m), load);
  const done = posted[posted.length - 1] as { done: { events: number; of: number } };
  assert.deepEqual([done.done.events, done.done.of], [1500, 2500]);
  assert.ok(posted.slice(0, -1).every((m) => "progress" in m));
  const failed: object[] = [];
  await handle("x", (m) => failed.push(m), async () => Promise.reject(new Error("no WebAssembly")));
  assert.deepEqual(failed, [{ error: "no WebAssembly" }]);
});

test("a log too short to cut is shared as it is, with no worker", async () => {
  const text = log(10).replace(`"v":1,`, `"v":1,"extra":[1],`);
  assert.deepEqual(await forSite(text, summarize(text).events), { text, events: 10, of: 10 });
});

test("a keyframe in a pasted log isn't counted as an event or a session", () => {
  const shared = runTrim(wasm, log(2500));
  const summary = summarize(shared.text);
  assert.equal(summary.events, 1500);
  assert.equal(summary.sessions, 15, "sessions 10 to 24, not the keyframe's");
  // Nor a line the site wouldn't read as an event.
  assert.equal(summarize('{"id":"x","type":"status","node":"x:s"}\n').events, 0);
  assert.equal(summarize('{"v":"1","id":"x","ts":"t","type":"status","node":"x:s"}\n').events, 0, "v not a number");
});
