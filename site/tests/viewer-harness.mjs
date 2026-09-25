// Loads the viewer page (src/view/assets: the same page `agent-graph view`
// serves and the site embeds) into jsdom, fed by a stub data source, so its
// behaviour can be tested without a browser. See the `agentGraphSource`
// interface at the top of app.js.

import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";

const assets = new URL("../../src/view/assets/", import.meta.url);
const html = readFileSync(new URL("index.html", assets), "utf8").replace(/<script[^>]*><\/script>/, "");
const app = readFileSync(new URL("app.js", assets), "utf8");

/**
 * The viewer, running against `source` (any methods not given are stubbed),
 * for the test `t`, which closes it at the end (its timers would otherwise
 * keep the test run alive). Returns jsdom's window; `window.__viewer`
 * exposes the page's state `S` and its navigation functions.
 */
export function loadViewer(t, source, { hash = "" } = {}) {
  const dom = new JSDOM(html, {
    url: `http://localhost/${hash}`,
    runScripts: "outside-only",
    pretendToBeVisual: true,
  });
  const { window } = dom;
  t.after(() => window.close());
  window.matchMedia = () => ({ matches: false, addEventListener() {} });
  window.HTMLElement.prototype.scrollIntoView = function () {
    window.__scrolls = (window.__scrolls || 0) + 1;
  };
  window.agentGraphSource = {
    liveLabel: "Live",
    imageUrl: null,
    info: async () => ({ now_ms: Date.now(), where: "test" }),
    subscribe() {},
    timeline: async () => ({ stops: [] }),
    ...source,
  };
  window.eval(
    `${app}\nwindow.__viewer = { S, goTo, goLive, selectRoot, selectNode, scheduleRefresh, renderAll };`,
  );
  return window;
}

/** Waits until `check()` is truthy (or fails the test after `ms`). */
export async function until(check, ms = 2000) {
  const start = Date.now();
  for (;;) {
    const value = check();
    if (value) return value;
    if (Date.now() - start > ms) throw new Error(`timed out waiting for ${check}`);
    await new Promise((r) => setTimeout(r, 5));
  }
}

/** A node as the reducer would send it, with `fields` over the defaults. */
export function node(id, fields = {}) {
  return {
    id,
    kind: id.includes("/") ? "agent" : "session",
    provider: id.split(":")[0],
    children: [],
    state: "working",
    tasks: [],
    spawns: [],
    waits: [],
    messages: [],
    open_tasks: 0,
    stale: false,
    started_at: "2026-09-25T10:00:00.000Z",
    last_event_at: new Date().toISOString(),
    ...fields,
  };
}

/** A graph of `nodes`, rooted at those without a parent. */
export function graph(nodes, fields = {}) {
  const byId = Object.fromEntries(nodes.map((n) => [n.id, n]));
  return {
    at: null,
    events: nodes.length,
    nodes: byId,
    roots: nodes.filter((n) => !n.parent).map((n) => n.id),
    ...fields,
  };
}
