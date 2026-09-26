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
export function loadViewer(t, source, { hash = "", path = "", fetch, EventSource } = {}) {
  const dom = new JSDOM(html, {
    url: `http://localhost:7777/${path}${hash}`,
    runScripts: "outside-only",
    pretendToBeVisual: true,
  });
  const { window } = dom;
  t.after(() => window.close());
  window.matchMedia = () => ({ matches: false, addEventListener() {} });
  window.HTMLElement.prototype.scrollIntoView = function () {
    window.__scrolls = (window.__scrolls || 0) + 1;
  };
  // With no `source`, the page uses its own (the local viewer's API), over
  // the given `fetch` and `EventSource`.
  if (fetch) window.fetch = fetch;
  if (EventSource) window.EventSource = EventSource;
  if (source) {
    // The page doesn't ask for a timeline: the graph now carries it. A stub
    // gives it as `timeline(root)`.
    const { timeline = async () => ({ stops: [] }), ...rest } = source;
    window.agentGraphSource = {
      liveLabel: "Live",
      imageUrl: null,
      info: async () => ({ now_ms: Date.now(), where: "test" }),
      subscribe() {},
      ...rest,
      // Replies as the server's do: the tree asked for (see `asServer`),
      // and for the graph now, its timeline.
      ...(source.graph
        ? {
            graph: async (until, root) => {
              const g = asServer(await source.graph(until, root), root);
              if (!until) g.stops = g.root ? (await timeline(g.root)).stops : [];
              return g;
            },
          }
        : {}),
    };
  }
  window.eval(
    `${app}\nwindow.__viewer = { S, goTo, goLive, selectRoot, selectNode, scheduleRefresh, renderAll, CACHED_STEPS };`,
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

/** What names a node and how it's doing: `timeline::Brief`. */
function brief(n) {
  const { id, kind, provider, parent, title, cwd, agent_type, attention, state, stale } = n;
  return { id, kind, provider, parent, title, cwd, agent_type, attention, state, stale };
}

/**
 * `g` (from `graph`) as the server replies to a request for `root`'s tree:
 * `root` echoed (or the newest session, if it's not in the graph), `nodes`
 * that tree, and `others` what names the nodes it refers to. Done to `g`
 * itself, so tests can compare what the page holds with what they gave it;
 * all its nodes are kept in `g.all`.
 */
export function asServer(g, root) {
  if (g.others && !g.all) return g; // already a server's reply
  g.all ??= g.nodes;
  const tree = root && g.all[root] ? root : g.roots[0] || null;
  const nodes = {};
  const queue = tree ? [tree] : [];
  while (queue.length) {
    const n = g.all[queue.shift()];
    if (!n || nodes[n.id]) continue;
    nodes[n.id] = n;
    queue.push(...n.children);
  }
  const others = {};
  for (const n of Object.values(nodes)) {
    const referred = [
      ...((n.blocked && n.blocked.on) || []),
      ...n.messages.map((m) => m.peer),
      n.parent,
    ];
    for (const id of referred) if (id && !nodes[id] && g.all[id]) others[id] = brief(g.all[id]);
  }
  return Object.assign(g, { root: tree, nodes, others });
}

/**
 * A graph of `nodes`, rooted at those without a parent: every node, and a
 * summary of each session with just what the server's has
 * (`timeline::SessionSummary`). The harness hands it to the page as the
 * server would (`asServer`).
 */
export function graph(nodes, fields = {}) {
  const byId = Object.fromEntries(nodes.map((n) => [n.id, n]));
  const roots = nodes.filter((n) => !n.parent || !byId[n.parent]).map((n) => n.id);
  const tree = (id) => {
    const out = [];
    const queue = [id];
    while (queue.length) {
      const n = byId[queue.shift()];
      if (!n || out.includes(n)) continue;
      out.push(n);
      queue.push(...n.children);
    }
    return out;
  };
  const sessions = Object.fromEntries(
    roots.map((id) => {
      const root = byId[id];
      const all = tree(id);
      const first = (f) => {
        const n = all.find(f);
        return n ? brief(n) : null;
      };
      return [
        id,
        {
          ...brief(root),
          last_event_at: root.last_event_at,
          started_at: root.started_at,
          headline: root.headline,
          tasks: root.tasks.length,
          open_tasks: root.open_tasks,
          agents: all.length - 1,
          needs_you: first((n) => n.state === "input_required"),
          deadlocked: all.some((n) => n.blocked && n.blocked.cycle),
          stuck: first((n) => n.stale),
          busy: all.some((n) => n.state === "working"),
        },
      ];
    }),
  );
  return {
    at: null,
    events: nodes.length,
    nodes: byId,
    roots,
    sessions,
    ...fields,
  };
}
