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
