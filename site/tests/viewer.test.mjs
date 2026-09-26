// The viewer's behaviour, in jsdom. See viewer-harness.mjs.

import assert from "node:assert/strict";
import { test } from "node:test";

import { graph, loadViewer, node, until } from "./viewer-harness.mjs";

test("R3: a tree with a loop in it is drawn once, and the page keeps working", async (t) => {
  const a = node("x:a", { children: ["x:b"] });
  const b = node("x:b", { parent: "x:a", children: ["x:a"] });
  const g = graph([a, b], { roots: ["x:a"] });
  const window = loadViewer(t, { graph: async () => g });
  const doc = window.document;
  await until(() => doc.querySelectorAll("#view .node").length || !doc.querySelector("#banner").hidden);
  assert.equal(doc.querySelector("#banner").hidden, true, doc.querySelector("#banner").textContent);
  assert.equal(doc.querySelectorAll("#view .node").length, 2);
});

test("R7: the local page takes its key from the link, keeps it, and sends it", async (t) => {
  const requests = [];
  const streams = [];
  const g = graph([node("x:a")]);
  const fetch = async (url, init = {}) => {
    requests.push({ url: String(url), key: init.headers && init.headers["X-Agent-Graph-Key"] });
    const body = String(url).startsWith("/api/timeline")
      ? { stops: [] }
      : String(url).startsWith("/api/info")
        ? { now_ms: Date.now(), events_dir: "x" }
        : g;
    return { ok: true, status: 200, json: async () => body, text: async () => "", blob: async () => ({}) };
  };
  class EventSource {
    constructor(url) {
      streams.push(url);
    }
    addEventListener() {}
  }
  const window = loadViewer(t, null, { path: "?key=k3y", hash: "#x:a", fetch, EventSource });
  await until(() => window.document.querySelectorAll("#view .node").length);

  assert.equal(window.location.search, "", "the key is out of the address bar");
  assert.equal(window.location.hash, "#x:a", "the rest of the link is kept");
  assert.equal(window.localStorage.getItem("agentGraphKey"), "k3y");
  assert.ok(requests.length > 0);
  for (const r of requests) assert.equal(r.key, "k3y", r.url);
  assert.deepEqual(streams, ["/api/stream?key=k3y"]);

  // Saving an image fetches it with the key as a header: the key is never
  // in a URL, so it can't end up in the saved file's "where from" details.
  const image = window.document.querySelector('a[href^="/api/image.png"]');
  assert.ok(image, "a Save image link");
  assert.ok(!image.getAttribute("href").includes("k3y"), image.getAttribute("href"));
  window.URL.createObjectURL = () => "blob:saved";
  window.URL.revokeObjectURL = () => {};
  const before = requests.length;
  image.click();
  await until(() => requests.length > before);
  const fetched = requests.slice(before).find((r) => r.url.startsWith("/api/image.png"));
  assert.ok(fetched, "fetched the image");
  assert.equal(fetched.key, "k3y");
  assert.ok(!fetched.url.includes("k3y"));
});

/** A promise, and the function that settles it. */
function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const settle = () => new Promise((r) => setTimeout(r, 80));

/** Timeline stops with these ids. */
const stopsOf = (ids) =>
  ids.map((id, i) => ({ id, ts: `2026-09-25T10:00:0${i}.000Z`, label: `step ${id}`, category: "status" }));

test("R20: a step's graph arriving late never replaces the graph shown since", async (t) => {
  let live = graph([node("x:a"), node("x:b")]);
  const cachedStep = graph([node("x:a")]);
  const late = graph([node("x:a"), node("x:b")]);
  let stops = stopsOf(["e1", "e2", "e3", "e4", "e5", "e6", "e7", "e8"]);
  // Every step but e2 waits until the test lets it through; so does the
  // timeline, while `hold` is set.
  const pending = [];
  const calls = {};
  let hold = null;
  const window = loadViewer(
    t,
    {
      graph: async (at) => {
        if (!at) return live;
        calls[at] = (calls[at] || 0) + 1;
        if (at === "e2") return cachedStep;
        const d = deferred();
        pending.push(d);
        return d.promise;
      },
      timeline: async () => {
        if (hold) await hold.promise;
        return { stops };
      },
    },
    { hash: "#x:a" },
  );
  const v = window.__viewer;
  const banner = window.document.querySelector("#banner");
  await until(() => v.S.stops.length === stops.length);
  const loading = async (pos) => {
    const before = pending.length;
    v.goTo(pos);
    await until(() => pending.length > before);
    return pending[pending.length - 1];
  };
  const holding = () => {
    hold = deferred();
    return () => {
      hold.resolve();
      hold = null;
    };
  };

  // Going live while a step loads.
  let slow = await loading(0);
  v.goLive();
  slow.resolve(late);
  await settle();
  assert.equal(v.S.shown, v.S.live, "went live");

  // Moving to a cached step while another loads.
  v.goTo(1);
  await until(() => v.S.shown === cachedStep);
  slow = await loading(2);
  v.goTo(1);
  slow.resolve(late);
  await settle();
  assert.equal(v.S.shown, cachedStep, "the cached step");

  // The session goes, so a refresh goes live on another, while a step
  // loads: the step arrives before the new session's timeline does, and
  // the old session's steps can't be stepped to meanwhile.
  slow = await loading(3);
  live = graph([node("x:b"), node("x:c")]);
  let release = holding();
  v.scheduleRefresh();
  await until(() => v.S.root === "x:b");
  slow.resolve(late);
  await settle();
  assert.equal(v.S.shown, v.S.live, "live, not the step of the session that went");
  assert.equal(window.document.querySelector("#step-count").textContent, "No events yet", "and the page says so");
  assert.equal(window.document.querySelector("#prev").disabled, true, "with nothing to step back to");
  v.goTo(1);
  await settle();
  assert.equal(v.S.shown, v.S.live, "not a step of the session that went");
  release();
  const refreshed = () => v.S.following && v.S.stops.length && v.S.pos === v.S.stops.length - 1;
  await until(refreshed);
  assert.equal(v.S.shown, v.S.live, "the other session, live");
  // Nor was it kept for later: it's from before the refresh.
  v.goTo(3);
  await settle();
  assert.equal(calls.e4, 2, "fetched again, not taken from the cache");
  pending.at(-1).resolve(late);
  await settle();
  assert.equal(v.S.shown, late);

  // Choosing another session while a step loads: the step arrives before
  // the new session's timeline does.
  slow = await loading(4);
  release = holding();
  v.selectRoot("x:c");
  slow.resolve(late);
  await settle();
  assert.notEqual(v.S.shown, late, "the step of the session left");
  release();
  await until(() => v.S.root === "x:c" && refreshed());
  assert.equal(v.S.shown, v.S.live);

  // Choosing another session from a past step: live straight away, not
  // the step of the session left.
  v.goTo(1);
  await until(() => v.S.shown === cachedStep);
  release = holding();
  v.selectRoot("x:b");
  assert.equal(v.S.shown, v.S.live, "before its timeline comes");
  v.goTo(1);
  await settle();
  assert.equal(v.S.shown, v.S.live, "not a step of the session left");
  release();
  await until(() => v.S.root === "x:b" && refreshed());

  // A refresh that stays on the step being loaded still shows it.
  slow = await loading(5);
  stops = [...stops, ...stopsOf(["e9"])];
  v.scheduleRefresh();
  await until(() => v.S.stops.length === stops.length);
  const step = graph([node("x:b")]);
  slow.resolve(step);
  await settle();
  assert.equal(v.S.pos, 5);
  assert.equal(v.S.shown, step, "the step, once loaded");

  // A step left behind that fails says nothing; the step being viewed does.
  slow = await loading(6);
  v.goLive();
  slow.reject(new Error("503 busy"));
  await settle();
  assert.equal(banner.hidden, true, banner.textContent);
  slow = await loading(6);
  slow.reject(new Error("503 busy"));
  await settle();
  assert.equal(banner.hidden, false, "the step being viewed failed");
});

test("R21: stepping while a refresh is in flight isn't undone", async (t) => {
  const live = graph([node("x:a")]);
  const step = graph([node("x:a")]);
  let stops = stopsOf(["e1", "e2", "e3"]);
  let hold = null;
  const window = loadViewer(
    t,
    {
      graph: async (at) => (at ? step : live),
      timeline: async () => {
        if (hold) await hold.promise;
        return { stops };
      },
    },
    { hash: "#x:a" },
  );
  const v = window.__viewer;
  await until(() => v.S.stops.length === stops.length);
  assert.equal(v.S.following, true);

  // The refresh brings a new event, while the user steps back to the first.
  hold = deferred();
  v.scheduleRefresh();
  await settle();
  v.goTo(0);
  await until(() => v.S.shown === step);
  stops = stopsOf(["e1", "e2", "e3", "e4"]);
  hold.resolve();
  await until(() => v.S.stops.length === 4);
  assert.equal(v.S.pos, 0, "still where it was moved to");
  assert.equal(v.S.following, false);
  assert.equal(v.S.shown, step);
  assert.equal(window.document.querySelector("#step-count").textContent, "Step 1 of 4");
});

test("R21: when the step being viewed leaves the timeline, the nearest one before it is shown", async (t) => {
  const live = graph([node("x:a")]);
  const ids = ["e1", "e2", "e3", "e4", "e5", "e6", "e7", "e8"];
  const graphs = Object.fromEntries(ids.map((id) => [id, graph([node("x:a")])]));
  const all = stopsOf(ids);
  const without = all.filter((s) => !["e3", "e4", "e5"].includes(s.id));
  let stops = all;
  let slow = null;
  const calls = {};
  const window = loadViewer(
    t,
    {
      graph: async (at) => {
        if (!at) return live;
        calls[at] = (calls[at] || 0) + 1;
        if (at === "e5" && slow) return slow.promise;
        return graphs[at];
      },
      timeline: async () => ({ stops }),
    },
    { hash: "#x:a" },
  );
  const v = window.__viewer;
  const count = () => window.document.querySelector("#step-count").textContent;
  await until(() => v.S.stops.length === all.length);
  v.goTo(4);
  await until(() => v.S.shown === graphs.e5);

  // e3 to e5 go (they moved to another tree, say): back to e2, still in the
  // past, not on to e8 (live).
  stops = without;
  v.scheduleRefresh();
  await until(() => v.S.stops.length === without.length && v.S.shown === graphs.e2);
  assert.equal(v.S.pos, 1);
  assert.equal(v.S.following, false);
  assert.equal(count(), "Step 2 of 5");

  // Again while e5 is still loading: it isn't shown under e2's label.
  stops = all;
  v.scheduleRefresh();
  await until(() => v.S.stops.length === all.length);
  slow = deferred();
  v.goTo(4);
  await settle();
  assert.equal(calls.e5, 2, "e5 is loading");
  stops = without;
  v.scheduleRefresh();
  await until(() => v.S.stops.length === without.length && v.S.shown === graphs.e2);
  slow.resolve(graphs.e5);
  await settle();
  assert.equal(v.S.shown, graphs.e2, "e2's graph under e2's label");

  // Nothing left, while a past step is still loading: live, and the step
  // isn't shown when it comes.
  stops = all;
  v.scheduleRefresh();
  await until(() => v.S.stops.length === all.length);
  slow = deferred();
  v.goTo(4);
  await settle();
  assert.equal(calls.e5, 3, "e5 is loading");
  stops = [];
  v.scheduleRefresh();
  await until(() => v.S.following);
  slow.resolve(graphs.e5);
  await settle();
  assert.equal(v.S.shown, v.S.live);
  assert.equal(count(), "No events yet");
});

test("R21: a refresh overtaken by a change of session shows nothing of the old one's timeline", async (t) => {
  const live = graph([node("x:a"), node("x:b")]);
  const stopsFor = { "x:a": stopsOf(["a1", "a2", "a3"]), "x:b": stopsOf(["b1", "b2"]) };
  const holds = {};
  const window = loadViewer(
    t,
    {
      graph: async () => live,
      timeline: async (root) => {
        if (holds[root]) await holds[root].promise;
        return { stops: stopsFor[root] };
      },
    },
    { hash: "#x:a" },
  );
  const v = window.__viewer;
  const count = () => window.document.querySelector("#step-count").textContent;
  const banner = window.document.querySelector("#banner");
  await until(() => v.S.stops.length === 3);
  assert.equal(count(), "Step 3 of 3");

  // The overtaken refresh comes back: x:a's timeline isn't shown under x:b.
  holds["x:a"] = deferred();
  holds["x:b"] = deferred();
  v.scheduleRefresh();
  await settle();
  v.selectRoot("x:b");
  assert.equal(count(), "No events yet", "the page leaves x:a's timeline straight away");
  holds["x:a"].resolve();
  await settle();
  // (Array.from: the page's arrays are from its own realm.)
  assert.deepEqual(Array.from(v.S.stops, (s) => s.id), [], "not x:a's timeline under x:b");
  assert.equal(count(), "No events yet");
  holds["x:b"].resolve();
  await until(() => v.S.stops.length === 2);
  assert.deepEqual(Array.from(v.S.stops, (s) => s.id), ["b1", "b2"]);
  assert.equal(count(), "Step 2 of 2");

  // The overtaken refresh fails: nothing to say, it's no longer shown.
  holds["x:a"] = deferred();
  holds["x:b"] = deferred();
  v.scheduleRefresh();
  await settle();
  v.selectRoot("x:a");
  holds["x:b"].reject(new Error("404 gone"));
  await settle();
  assert.equal(banner.hidden, true, banner.textContent);
  holds["x:a"].resolve();
  await until(() => v.S.stops.length === 3);
  assert.equal(count(), "Step 3 of 3");
  assert.equal(banner.hidden, true, banner.textContent);
});

test("R21: choosing a session that has just gone is ignored, and the page recovers", async (t) => {
  let live = graph([node("x:a"), node("x:b")]);
  const stops = stopsOf(["a1", "a2"]);
  let hold = null;
  const window = loadViewer(
    t,
    {
      graph: async () => live,
      timeline: async () => {
        if (hold) await hold.promise;
        return { stops };
      },
    },
    { hash: "#x:a" },
  );
  const v = window.__viewer;
  const count = () => window.document.querySelector("#step-count").textContent;
  await until(() => v.S.stops.length === 2);

  // x:b goes; while the refresh that finds out waits for x:a's timeline,
  // x:b (still listed) is chosen.
  live = graph([node("x:a")]);
  hold = deferred();
  v.scheduleRefresh();
  await until(() => v.S.live === live);
  v.selectRoot("x:b");
  assert.equal(v.S.root, "x:a", "not a session that has gone");
  hold.resolve();
  hold = null;
  await settle();
  assert.equal(v.S.root, "x:a");
  assert.equal(count(), "Step 2 of 2");
  assert.equal(window.document.querySelector("#banner").hidden, true);
});

test("R21: a session's controls start afresh when it's chosen", async (t) => {
  const live = graph([node("x:a"), node("x:b")]);
  let hold = null;
  const window = loadViewer(
    t,
    {
      graph: async () => live,
      timeline: async () => {
        if (hold) await hold.promise;
        return { stops: stopsOf(["e1", "e2", "e3"]) };
      },
    },
    { hash: "#x:a" },
  );
  const v = window.__viewer;
  const $ = (sel) => window.document.querySelector(sel);
  await until(() => v.S.stops.length === 3);
  v.goTo(2);
  hold = deferred();
  v.selectRoot("x:b");
  assert.equal($("#prev").disabled, true, "nothing to go back to yet");
  assert.equal($("#next").disabled, true);
  assert.equal(v.S.pos, -1);
  hold.resolve();
  await until(() => v.S.stops.length === 3);
  assert.equal($("#step-count").textContent, "Step 3 of 3");
});
