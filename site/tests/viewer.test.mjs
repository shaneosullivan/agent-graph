// The viewer's behaviour, in jsdom. See viewer-harness.mjs.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import { asServer, graph, loadViewer, node, until } from "./viewer-harness.mjs";

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
        : asServer(g, new URL(String(url), "http://x").searchParams.get("root"));
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
  // R23: it asks for the tree of the session it shows.
  assert.ok(
    requests.some((r) => r.url === "/api/graph?root=x%3Aa"),
    requests.map((r) => r.url).join(", "),
  );
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

/** A source of `nodes` (sent as the server would), recording what it was asked for. */
function treeSource(nodes, stops = stopsOf(["e1", "e2"])) {
  const whole = graph(nodes);
  const asked = [];
  return {
    asked,
    source: {
      graph: async (until, root) => {
        asked.push({ until, root });
        return { ...whole, all: whole.nodes, at: until || null };
      },
      timeline: async () => ({ stops }),
    },
  };
}

test("R23: the page asks for the tree it shows, and lists the others from their summaries", async (t) => {
  const a = node("x:a", { cwd: "/w/a" });
  const b = node("x:b", { cwd: "/w/b", children: ["x:b/c"] });
  const c = node("x:b/c", { parent: "x:b", agent_type: "Explore", state: "input_required", attention: "May I?" });
  const { asked, source } = treeSource([a, b, c]);
  const window = loadViewer(t, source, { hash: "#x:a" });
  const v = window.__viewer;
  const doc = window.document;
  await until(() => v.S.stops.length === 2);
  assert.deepEqual({ ...asked[0] }, { until: null, root: "x:a" }, "the session in the address");

  // x:b's tree isn't here, but the list says what's going on in it.
  assert.equal(v.S.live.nodes["x:b"], undefined);
  const items = [...doc.querySelectorAll("#session-list .session")];
  assert.equal(items.length, 2);
  const itemB = items.find((i) => i.textContent.includes("b"));
  assert.match(itemB.textContent, /Needs you: May I\?/);
  assert.match(itemB.textContent, /1 agent/);

  // Choosing it shows what its summary says while its tree comes...
  itemB.click();
  assert.match(doc.querySelector("#view h1").textContent, /b/);
  assert.equal(doc.querySelector("#view .empty-note").textContent, "Loading…");
  // ...and asks for its tree; steps ask for it too.
  await until(() => v.S.root === "x:b" && v.S.live.nodes["x:b/c"]);
  assert.deepEqual({ ...asked.at(-1) }, { until: null, root: "x:b" });
  v.goTo(0);
  await until(() => asked.at(-1).until === "e1");
  assert.deepEqual({ ...asked.at(-1) }, { until: "e1", root: "x:b" });
  await until(() => doc.querySelectorAll("#view .node").length === 2);
});

test("R23: with no session named, one request brings the newest session's tree", async (t) => {
  const { asked, source } = treeSource([node("x:a")]);
  const window = loadViewer(t, source);
  const v = window.__viewer;
  await until(() => v.S.root === "x:a" && window.document.querySelectorAll("#view .node").length === 1);
  assert.deepEqual(
    asked.map((r) => ({ ...r })),
    [{ until: null, root: null }],
  );
});

/** x:a waits on x:c, a session of its own, and on its agent x:c/d. */
function waitingOnAnother() {
  const wait = (on) => ({ wait_id: on, on, open: true, spawn: false, started_at: "2026-09-25T10:00:00.000Z" });
  const a = node("x:a", {
    waits: [wait("x:c"), wait("x:c/d")],
    blocked: { on: ["x:c", "x:c/d"], starting: 0, nodes: 2, open_tasks: 0, cycle: false },
  });
  const c = node("x:c", { cwd: "/w/c", children: ["x:c/d"] });
  const d = node("x:c/d", { parent: "x:c", agent_type: "Explore", background: true, stale: true });
  // A session of its own, unrelated.
  const z = node("x:z", { children: ["x:z/q"] });
  const q = node("x:z/q", { parent: "x:z", agent_type: "Plan" });
  return [a, c, d, z, q];
}

test("R23: a node outside the tree is named, and opening it shows its own tree", async (t) => {
  const { asked, source } = treeSource(waitingOnAnother());
  let hold = null;
  const held = {
    ...source,
    graph: async (until, root) => {
      const g = source.graph(until, root);
      if (hold && root === "x:c/d") await hold.promise;
      return g;
    },
  };
  const window = loadViewer(t, held, { hash: "#x:a" });
  const v = window.__viewer;
  const doc = window.document;
  await until(() => v.S.stops.length === 2);
  assert.equal(v.S.live.nodes["x:c"], undefined, "not x:a's tree");
  assert.equal(v.S.live.others["x:c"].state, "working", "only what names it");
  // How it's doing, from what names it.
  assert.match(doc.querySelector("#view .node-blocked").textContent, /\(looks stuck\)/);

  const links = () => [...doc.querySelectorAll("#detail button.linkish")];
  // An agent of the other session (not in the sessions list): its own tree.
  const agent = links().find((b) => b.textContent.startsWith("Explore"));
  assert.ok(agent, doc.querySelector("#detail").textContent);
  assert.ok(agent.closest("li").querySelector(".dot.working"), "its state");
  hold = deferred();
  agent.click();
  // Named at once, while its tree comes.
  assert.match(doc.querySelector("#view h1").textContent, /^Explore/);
  hold.resolve();
  hold = null;
  await until(() => v.S.root === "x:c/d" && v.S.live.root === "x:c/d");
  assert.deepEqual({ ...asked.at(-1) }, { until: null, root: "x:c/d" });

  // The other session itself.
  v.selectRoot("x:a");
  await until(() => v.S.root === "x:a" && v.S.live.root === "x:a" && links().length);
  links()
    .find((b) => b.textContent === "c")
    .click();
  await until(() => v.S.root === "x:c" && v.S.live.nodes["x:c/d"]);
  assert.deepEqual({ ...asked.at(-1) }, { until: null, root: "x:c" });
});

test("R23: an address naming a node this graph doesn't have shows its tree", async (t) => {
  const { source } = treeSource(waitingOnAnother());
  const window = loadViewer(t, source, { hash: "#x:a" });
  const v = window.__viewer;
  await until(() => v.S.stops.length === 2);
  assert.equal(v.S.live.nodes["x:z/q"] || v.S.live.others["x:z/q"], undefined);
  window.location.hash = "#x:z/q";
  await until(() => v.S.root === "x:z/q" && v.S.live.root === "x:z/q");
  assert.equal(window.document.querySelectorAll("#view .node").length, 1);
});

test("R23: an address naming a node that doesn't exist goes back to where it was", async (t) => {
  // The newest session (the one a reply falls back to) refers to neither.
  const [a, c, d, z, q] = waitingOnAnother();
  const { source } = treeSource([z, q, a, c, d]);
  const window = loadViewer(t, source, { hash: "#x:c" });
  const v = window.__viewer;
  await until(() => v.S.stops.length === 2 && v.S.root === "x:c");
  window.location.hash = "#x:typo";
  await until(() => v.S.root === "x:c" && v.S.live.root === "x:c" && window.location.hash === "#x%3Ac");
  await settle();
  assert.equal(v.S.root, "x:c", "not the newest session");

  // From an agent of a session that isn't the newest, too.
  window.location.hash = "#x:c/d";
  await until(() => v.S.root === "x:c/d" && v.S.live.root === "x:c/d");
  window.location.hash = "#x:typo";
  await until(() => v.S.root === "x:c/d" && v.S.live.root === "x:c/d" && window.location.hash === "#x%3Ac%2Fd");
  await settle();
  assert.equal(v.S.root, "x:c/d");
});

test("R23: at a past step, a node outside the tree still opens its own tree", async (t) => {
  const [waiting, ...rest] = waitingOnAnother();
  const window = loadViewer(
    t,
    {
      // Waiting on x:c/d only at e1: the live graph doesn't have it.
      graph: async (at) => graph(at === "e1" ? [waiting, ...rest] : [node("x:a"), ...rest]),
      timeline: async () => ({ stops: stopsOf(["e1", "e2"]) }),
    },
    { hash: "#x:a" },
  );
  const v = window.__viewer;
  const doc = window.document;
  await until(() => v.S.stops.length === 2);
  assert.equal(v.S.live.others["x:c/d"], undefined);
  v.goTo(0);
  await until(() => [...doc.querySelectorAll("#detail button.linkish")].some((b) => b.textContent.startsWith("Explore")));
  [...doc.querySelectorAll("#detail button.linkish")].find((b) => b.textContent.startsWith("Explore")).click();
  await until(() => v.S.root === "x:c/d" && v.S.live.root === "x:c/d");
});

test("R23: an old session named in the address is shown", async (t) => {
  const old = new Date(Date.now() - 3 * 24 * 3600 * 1000).toISOString();
  const { source } = treeSource([node("x:a", { last_event_at: old }), node("x:b")]);
  const window = loadViewer(t, source, { hash: "#x:a" });
  await until(() => window.document.querySelectorAll("#view .node").length === 1);
  assert.equal(window.__viewer.S.root, "x:a");
});

test("R23: a refresh overtaken while its graph comes shows nothing of it", async (t) => {
  const { source } = treeSource(waitingOnAnother());
  let hold = null;
  const held = {
    ...source,
    graph: async (until, root) => {
      const g = source.graph(until, root);
      if (hold && !until) await hold.promise;
      return g;
    },
  };
  const window = loadViewer(t, held, { hash: "#x:a" });
  const v = window.__viewer;
  await until(() => v.S.stops.length === 2);

  hold = deferred();
  v.scheduleRefresh();
  await settle();
  v.selectRoot("x:c");
  const release = hold;
  hold = null;
  release.resolve();
  await settle();
  // Not x:a's tree taken for x:c's (which would send the page back to x:a).
  assert.equal(v.S.root, "x:c");
  await until(() => v.S.live.root === "x:c" && v.S.stops.length === 2);
  assert.equal(v.S.root, "x:c");
});

test("R23: a node of the tree that hadn't started at a step says so", async (t) => {
  const a = node("x:a", { children: ["x:a/b"] });
  const b = node("x:a/b", { parent: "x:a", agent_type: "Explore" });
  const window = loadViewer(
    t,
    {
      graph: async (at) => (at === "e1" ? graph([node("x:a")]) : graph([a, b])),
      timeline: async () => ({ stops: stopsOf(["e1", "e2"]) }),
    },
    { hash: "#x:a" },
  );
  const v = window.__viewer;
  await until(() => v.S.stops.length === 2);
  v.selectNode("x:a/b");
  v.goTo(0);
  await until(() => v.S.shown && v.S.shown.at === null && !v.S.shown.nodes["x:a/b"]);
  assert.match(window.document.querySelector("#detail").textContent, /hadn’t started yet/);
});

test("R23: with no session named and none recent, none is shown", async (t) => {
  const old = new Date(Date.now() - 3 * 24 * 3600 * 1000).toISOString();
  const { source } = treeSource([node("x:a", { last_event_at: old })]);
  const window = loadViewer(t, source);
  await until(() => window.document.querySelector("#view h2"));
  assert.equal(window.__viewer.S.root, null);
  assert.equal(window.document.querySelector("#view h2").textContent, "Nothing in the last 24 hours");
});

test("R34: only the most recently shown steps' graphs are kept", async (t) => {
  const ids = Array.from({ length: 50 }, (_, i) => `e${i}`);
  const stops = ids.map((id, i) => ({
    id,
    ts: new Date(Date.UTC(2026, 8, 25, 10, 0, i)).toISOString(),
    label: id,
    category: "status",
  }));
  const calls = {};
  const window = loadViewer(
    t,
    {
      graph: async (at) => {
        if (at) calls[at] = (calls[at] || 0) + 1;
        return { ...graph([node("x:a")]), at: at || null };
      },
      timeline: async () => ({ stops }),
    },
    { hash: "#x:a" },
  );
  const v = window.__viewer;
  await until(() => v.S.stops.length === stops.length);
  const show = async (pos) => {
    v.goTo(pos);
    await until(() => v.S.shown && v.S.shown.at === ids[pos]);
  };
  assert.ok(v.CACHED_STEPS < ids.length - 2, "more steps than are kept");
  for (let pos = 0; pos < ids.length - 1; pos++) await show(pos);
  assert.ok(v.S.cache.size <= v.CACHED_STEPS, `${v.S.cache.size} kept`);

  // The first was shown long ago: it's asked for again. Going back to one
  // shown lately keeps it among the most recent, as the others go.
  await show(0);
  assert.equal(calls.e0, 2, "asked for again");
  await show(ids.length - 2);
  for (let pos = 1; pos < v.CACHED_STEPS; pos++) await show(pos);
  await show(ids.length - 2);
  assert.equal(calls[ids.at(-2)], 1, "kept, as it was shown lately");
  assert.ok(v.S.cache.size <= v.CACHED_STEPS, `${v.S.cache.size} kept`);
});

test("R35: the tree scrolls to the ringed card when the step changes, not whenever it's drawn", async (t) => {
  const a = node("x:a", { children: ["x:a/b"] });
  const b = node("x:a/b", { parent: "x:a", agent_type: "Explore" });
  let stops = stopsOf(["e1", "e2", "e3"]).map((s) => ({ ...s, node: "x:a/b" }));
  const window = loadViewer(
    t,
    {
      graph: async (at) => ({ ...graph([a, b]), at: at || null }),
      timeline: async () => ({ stops }),
    },
    { hash: "#x:a" },
  );
  const v = window.__viewer;
  const scrolls = () => window.__scrolls || 0;
  await until(() => v.S.stops.length === 3);
  v.goTo(0);
  await until(() => window.document.querySelector("#view .node.current"));
  assert.equal(scrolls(), 1, "to the step's card");

  // Drawn again: a card chosen, new events. The page stays where it's been scrolled to.
  v.selectNode("x:a");
  stops = [...stops, ...stopsOf(["e4"])];
  v.scheduleRefresh();
  await until(() => v.S.stops.length === 4);
  v.renderAll();
  assert.equal(window.document.querySelector("#view .node.current").dataset.id, "x:a/b");
  assert.equal(scrolls(), 1, "not scrolled back");

  v.goTo(1);
  await until(() => v.S.shown.at === "e2");
  assert.equal(scrolls(), 2, "to the next step's card");
});

test("R36: an address that isn't a well-formed one names nothing, and the page still works", async (t) => {
  const { source } = treeSource([node("x:a"), node("x:b")]);
  const window = loadViewer(t, source, { hash: "#x%3Ab%E0%A4%A" });
  const v = window.__viewer;
  const banner = window.document.querySelector("#banner");
  await until(() => v.S.stops.length === 2 || !banner.hidden);
  assert.equal(banner.hidden, true, banner.textContent);
  assert.equal(v.S.root, "x:a", "the newest session");

  // Changed to one in the page: nothing happens.
  window.location.hash = "#%";
  await settle();
  assert.equal(banner.hidden, true, banner.textContent);
  assert.equal(v.S.root, "x:a");
  window.location.hash = "#x%3Ab";
  await until(() => v.S.root === "x:b" && v.S.live.root === "x:b");
});

test("R37: long unbroken text wraps in cards and panels", async (t) => {
  const long = (what) => `${what}-${"x".repeat(300)}`;
  const a = node("x:a", { title: long("title"), cwd: `/${long("folder")}`, headline: long("headline"), children: ["x:a/b"] });
  const b = node("x:a/b", {
    parent: "x:a",
    agent_type: long("type"),
    purpose: long("purpose"),
    headline: long("headline"),
    state: "input_required",
    attention: long("attention"),
    messages: [{ direction: "received", peer: "x:a", ts: a.started_at, summary: long("summary"), body: long("body") }],
  });
  const window = loadViewer(t, { graph: async () => graph([a, b]) }, { hash: "#x:a" });
  const doc = window.document;
  const style = doc.createElement("style");
  style.textContent = readFileSync(new URL("../../src/view/assets/app.css", import.meta.url), "utf8");
  doc.head.append(style);
  await until(() => doc.querySelectorAll("#view .node").length === 2);
  window.__viewer.selectNode("x:a/b");

  // Whether it may break anywhere: from its own style or (as overflow-wrap
  // is inherited) the nearest that sets it. jsdom doesn't inherit it itself.
  const wraps = (el) => {
    for (let e = el; e; e = e.parentElement) {
      const value = window.getComputedStyle(e).overflowWrap;
      if (value && value !== "normal") return value === "anywhere" || value === "break-word";
    }
    return false;
  };
  const texts = [...doc.querySelectorAll("#view *, #detail *")].filter((el) =>
    [...el.childNodes].some((k) => k.nodeType === 3 && /x{300}/.test(k.data)),
  );
  assert.ok(texts.length >= 10, `${texts.length} texts`);
  for (const el of texts) {
    // Cut short with an ellipsis instead is fine too.
    const cut = (e) => e && (window.getComputedStyle(e).textOverflow === "ellipsis" || cut(e.parentElement));
    assert.ok(wraps(el) || cut(el), `${el.className || el.tagName}: ${el.textContent.slice(0, 20)}`);
  }
});
