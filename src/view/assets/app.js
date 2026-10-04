'use strict';

// The whole viewer is in this one block, so nothing it names becomes a
// global (a script's top-level functions are properties of `window`
// otherwise): `window.nodeName` from the function below, say, breaks
// React's event handling on the site's pages (in development, where Next
// reads it). In strict mode a block's functions are its own.
{

// Agent Graph viewer. Event text (task names, summaries, messages) comes from
// models, so it is only ever inserted as text, never as HTML.

const STATE_LABEL = {
  working: 'Working',
  input_required: 'Needs you',
  idle: 'Idle',
  completed: 'Completed',
  failed: 'Failed',
  canceled: 'Canceled',
};
const RECENT_MS = 24 * 60 * 60 * 1000;
const THUMB = 18; // slider thumb width, px; matches app.css
const CACHED_STEPS = 32; // past steps' graphs kept, the most recently shown

const S = {
  live: null, // the graph now
  shown: null, // the graph being displayed (live, or at a past stop)
  root: null, // selected session id
  selected: null, // selected node id (detail panel)
  stops: [], // timeline for `root`
  pos: -1, // index into stops
  following: true, // stay on the newest stop as events arrive
  showAll: false,
  showDone: savedShowDone(), // show finished sessions and agents
  hiddenApps: savedHiddenApps(), // apps whose sessions the list leaves out (see `renderAppFilter`)
  collapsed: savedCollapsed(), // folders whose sessions the list folds away (see `folderGroups`)
  connected: false,
  info: null,
  cache: new Map(), // event id -> graph at that stop, least recently shown first (a new map on each refresh)
  seq: 0, // bumped whenever the view moves on, so a step still loading isn't shown
  lastFlashed: null,
  scrolledTo: null, // what the ringed card last brought into view stands for (see `renderMain`)
  error: null,
  skew: 0, // server clock minus ours; non-zero when AGENT_GRAPH_NOW pins it
  opened: null, // { id, busy?, error?, command? }: the last Open button press
  returnTo: null, // where to go back to if a node named in the address doesn't exist
  attentionAt: null, // the session needing you that the counter's arrows last scrolled to
  showNeedsYou: null, // a session whose node that needs you is to be brought into view and pulsed, once drawn (see `stepAttention`)
  view: savedViewMode(), // 'cards' or 'graph': how the tree's drawn (see the graph view)
  modal: null, // the node the details dialog shows, when it's open (see `openModal`)
  timelineOpen: false, // on a phone, whether the timeline drawer's up (see `setTimelineOpen`)
  legendOpen: false, // on a phone, whether the graph's legend is shown (see `legendButton`)
};

// ---------- helpers ----------

const $ = (sel) => document.querySelector(sel);

/**
 * Makes model-written text safe to show: control characters become spaces,
 * and bidirectional overrides (which make text display differently from what
 * it is) are dropped.
 */
const clean = (s) =>
  String(s)
    .replace(/[\u0000-\u001f\u007f-\u009f]/g, ' ')
    .replace(/[\u202a-\u202e\u2066-\u2069]/g, '');

/**
 * Builds an element. Strings become text nodes, never markup. Handlers are
 * set as properties (`onclick`), so `morph` can carry them over.
 */
function h(tag, props, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props || {})) {
    if (v == null || v === false) continue;
    if (k === 'class') el.className = v;
    else if (k.startsWith('on')) el[k] = v;
    else el.setAttribute(k, v === true ? '' : clean(v));
  }
  for (const kid of kids.flat(Infinity)) {
    if (kid == null || kid === false) continue;
    el.append(kid instanceof Node ? kid : clean(kid));
  }
  return el;
}

/**
 * Makes `el`'s children look like `kids`, as `replaceChildren(...kids)`
 * would, but keeps each node that's still there (the same tag, at the same
 * place), changing only what differs: its text, or its attributes and click
 * handler. So a redraw as events arrive leaves focus and selected text alone
 * wherever nothing changed. An element marked `data-keep` keeps whatever's in
 * it: something else draws that.
 */
function morph(el, kids) {
  const old = [...el.childNodes];
  kids.forEach((kid, i) => {
    const was = old[i];
    if (!was) el.append(kid);
    // One becoming (or no longer) kept for other code to draw in starts afresh.
    else if (was.nodeName !== kid.nodeName || (was.nodeType === Node.ELEMENT_NODE && was.hasAttribute('data-keep') !== kid.hasAttribute('data-keep'))) was.replaceWith(kid);
    else if (was.nodeType === Node.TEXT_NODE) {
      if (was.data !== kid.data) was.data = kid.data;
    } else {
      for (const { name } of [...was.attributes]) if (!kid.hasAttribute(name)) was.removeAttribute(name);
      for (const { name, value } of [...kid.attributes]) if (was.getAttribute(name) !== value) was.setAttribute(name, value);
      was.onclick = kid.onclick;
      // What's inside one marked data-keep is drawn by other code (the graph).
      if (!was.hasAttribute('data-keep')) morph(was, [...kid.childNodes]);
    }
  });
  for (const gone of old.slice(kids.length)) gone.remove();
}

/**
 * Redraws `el` with `kids` (see `morph`). A control that had focus and now
 * stands for something else, or has gone (a session listed above it moved
 * it down, say), gives it to the one that stands for what it did.
 */
function redraw(el, kids) {
  const focused = el.contains(document.activeElement) ? document.activeElement : null;
  const key = focused && controlKey(focused);
  morph(el, kids);
  if (key && controlKey(focused) !== key) {
    const again = [...el.querySelectorAll('[data-id]')].find((e) => controlKey(e) === key);
    if (again) again.focus({ preventScroll: true });
  }
}

/** What control `el` is, if it stands for a node: which node, and what it does with it. */
function controlKey(el) {
  return el.isConnected && el.dataset.id != null ? `${el.dataset.id} ${el.classList[0]}` : null;
}

/**
 * This run's key for `agent-graph view`'s API. It arrives in the link the
 * command printed (`?key=…`), is kept in this origin's storage (so another
 * tab, or a reload, still has it), and is taken out of the address bar.
 */
const KEY = (() => {
  if (window.agentGraphSource) return null;
  const params = new URLSearchParams(location.search);
  const given = params.get('key');
  try {
    if (given) localStorage.setItem('agentGraphKey', given);
  } catch {
    /* no storage: this page still has the key until it's closed */
  }
  if (given) {
    params.delete('key');
    const rest = params.toString();
    history.replaceState(null, '', `${location.pathname}${rest ? `?${rest}` : ''}${location.hash}`);
    return given;
  }
  try {
    return localStorage.getItem('agentGraphKey');
  } catch {
    return null;
  }
})();

/** `url` with the key added, for requests that can't send headers. */
const withKey = (url) => `${url}${url.includes('?') ? '&' : '?'}key=${encodeURIComponent(KEY || '')}`;

async function getJSON(url) {
  const res = await fetch(url, { cache: 'no-store', headers: { 'X-Agent-Graph-Key': KEY || '' } });
  if (res.status === 403) throw Object.assign(new Error('no key'), { needsKey: true });
  if (!res.ok) throw new Error(`${res.status} ${await res.text()}`);
  return res.json();
}

/**
 * Where the graph comes from. By default, the `agent-graph view` server's
 * API. A page can set `window.agentGraphSource` first to supply it another
 * way; the shared site computes it in the browser with the same reducer,
 * compiled to WebAssembly. Every method but `subscribe` returns a promise.
 */
const source = window.agentGraphSource || {
  /**
   * The graph now, or as of event `until`: a summary of every session
   * (`sessions`), and the nodes of the tree under `root` (`nodes`). The
   * graph now also has every stop in that tree's timeline (`stops`), worked
   * out with it, so a refresh is one request.
   */
  graph: (until, root) => {
    const params = new URLSearchParams();
    if (root) params.set('root', root);
    if (until) params.set('until', until);
    const query = params.toString();
    return getJSON(`/api/graph${query ? `?${query}` : ''}`);
  },
  /** `{ now_ms, where }`: the clock to measure "5m ago" by, and where events come from. */
  info: () => getJSON('/api/info').then((i) => ({ now_ms: i.now_ms, where: i.events_dir })),
  /** Calls `onChange()` when new events arrive and `onStatus(connected)` as the connection changes. */
  subscribe(onChange, onStatus) {
    const stream = new EventSource(withKey('/api/stream'));
    stream.addEventListener('open', () => onStatus(true));
    stream.addEventListener('changed', onChange);
    stream.addEventListener('error', () => onStatus(false));
  },
  /** A URL for a PNG of `root` (as of `until`), or null if images aren't available. */
  imageUrl(root, until, dark) {
    const params = new URLSearchParams({ root });
    if (until) params.set('until', until);
    if (dark) params.set('theme', 'dark');
    return `/api/image.png?${params}`;
  },
  /**
   * The image at `url` (from `imageUrl`), when fetching it takes more than a
   * link can carry: here, the key, sent as a header so it never ends up in
   * the saved file's "where from" details.
   */
  async fetchImage(url) {
    const res = await fetch(url, { cache: 'no-store', headers: { 'X-Agent-Graph-Key': KEY || '' } });
    if (!res.ok) throw new Error(`${res.status} ${await res.text()}`);
    return res.blob();
  },
  /** What to call the newest point in the timeline. */
  liveLabel: 'Live',
  /**
   * Reopens session `id` in its agent, in a new terminal window. Only a page
   * on the computer the sessions ran on can; a source without `open` offers
   * nothing to open. Rejects with an error that may carry a `command` to run.
   */
  async open(id) {
    const res = await fetch(`/api/open?node=${encodeURIComponent(id)}`, {
      method: 'POST',
      cache: 'no-store',
      headers: { 'X-Agent-Graph-Key': KEY || '' },
    });
    const body = await res.json().catch(() => ({ error: `${res.status} ${res.statusText}` }));
    if (!res.ok) throw Object.assign(new Error(body.error), { command: body.command });
    return body;
  },
};

const clamp = (n, lo, hi) => Math.max(lo, Math.min(hi, n));
const nowMs = () => Date.now() + S.skew;
const localId = (id) => id.split(/[/:]/).pop();
const short = (id) => localId(id).slice(0, 8);
const basename = (p) => (p ? p.split(/[\\/]/).filter(Boolean).pop() : null);

const PROVIDER_NAME = { 'claude-code': 'Claude Code', codex: 'Codex', gemini: 'Gemini CLI', cursor: 'Cursor' };
/** The app a session ran in, as the list names it. */
const appName = (provider) => PROVIDER_NAME[provider] || (provider === 'run' ? 'agent-graph run' : provider);

/** A 16×16 SVG, as an image the page's policy lets it show. */
const svgImage = (body) =>
  `data:image/svg+xml,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16">${body}</svg>`)}`;

/**
 * A small mark for each app, in its colours, to tell sessions apart at a
 * glance: simple drawings, but for Codex, OpenAI's own mark.
 */
const APP_ICON = {
  // A spark, in Claude's orange.
  'claude-code': svgImage(
    '<rect width="16" height="16" rx="4" fill="#D97757"/><path d="M8 3.2v9.6M3.2 8h9.6M4.6 4.6l6.8 6.8M11.4 4.6l-6.8 6.8" stroke="#fff" stroke-width="1.6" stroke-linecap="round"/>',
  ),
  // OpenAI's blossom (from openai.com's brand assets), black on white, as
  // the ChatGPT app's icon is.
  codex: svgImage(
    '<rect x="0.5" y="0.5" width="15" height="15" rx="3.5" fill="#fff" stroke="#d4d4d4"/><path transform="translate(8 8) scale(0.0449) translate(-280.29 -359.45)" fill="#000" d="M249.176 323.434V298.276C249.176 296.158 249.971 294.569 251.825 293.509L302.406 264.381C309.29 260.409 317.5 258.555 325.973 258.555C357.75 258.555 377.877 283.185 377.877 309.399C377.877 311.253 377.877 313.371 377.611 315.49L325.178 284.771C322.001 282.919 318.822 282.919 315.645 284.771L249.176 323.434ZM367.283 421.415V361.301C367.283 357.592 365.694 354.945 362.516 353.092L296.048 314.43L317.763 301.982C319.617 300.925 321.206 300.925 323.058 301.982L373.639 331.112C388.205 339.586 398.003 357.592 398.003 375.069C398.003 395.195 386.087 413.733 367.283 421.412V421.415ZM233.553 368.452L211.838 355.742C209.986 354.684 209.19 353.095 209.19 350.975V292.718C209.19 264.383 230.905 242.932 260.301 242.932C271.423 242.932 281.748 246.641 290.49 253.26L238.321 283.449C235.146 285.303 233.555 287.951 233.555 291.659V368.455L233.553 368.452ZM280.292 395.462L249.176 377.985V340.913L280.292 323.436L311.407 340.913V377.985L280.292 395.462ZM300.286 475.968C289.163 475.968 278.837 472.259 270.097 465.64L322.264 435.449C325.441 433.597 327.03 430.949 327.03 427.239V350.445L349.011 363.155C350.865 364.213 351.66 365.802 351.66 367.922V426.179C351.66 454.514 329.679 475.965 300.286 475.965V475.968ZM237.525 416.915L186.944 387.785C172.378 379.31 162.582 361.305 162.582 343.827C162.582 323.436 174.763 305.164 193.563 297.485V357.861C193.563 361.571 195.154 364.217 198.33 366.071L264.535 404.467L242.82 416.915C240.967 417.972 239.377 417.972 237.525 416.915ZM234.614 460.343C204.689 460.343 182.71 437.833 182.71 410.028C182.71 407.91 182.976 405.792 183.238 403.672L235.405 433.863C238.582 435.715 241.763 435.715 244.938 433.863L311.407 395.466V420.622C311.407 422.742 310.612 424.331 308.758 425.389L258.179 454.519C251.293 458.491 243.083 460.343 234.611 460.343H234.614ZM300.286 491.854C332.329 491.854 359.073 469.082 365.167 438.892C394.825 431.211 413.892 403.406 413.892 375.073C413.892 356.535 405.948 338.529 391.648 325.552C392.972 319.991 393.766 314.43 393.766 308.87C393.766 271.003 363.048 242.666 327.562 242.666C320.413 242.666 313.528 243.723 306.644 246.109C294.725 234.457 278.307 227.042 260.301 227.042C228.258 227.042 201.513 249.815 195.42 280.004C165.761 287.685 146.694 315.49 146.694 343.824C146.694 362.362 154.638 380.368 168.938 393.344C167.613 398.906 166.819 404.467 166.819 410.027C166.819 447.894 197.538 476.231 233.024 476.231C240.172 476.231 247.058 475.173 253.943 472.788C265.859 484.441 282.278 491.854 300.286 491.854Z"/>',
  ),
  // A four-pointed star, blue to violet.
  gemini: svgImage(
    '<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#4285F4"/><stop offset="1" stop-color="#9B72CB"/></linearGradient></defs><path d="M8 1c.6 3.6 3.4 6.4 7 7-3.6.6-6.4 3.4-7 7-.6-3.6-3.4-6.4-7-7 3.6-.6 6.4-3.4 7-7z" fill="url(#g)"/>',
  ),
  // A cube.
  cursor: svgImage(
    '<rect width="16" height="16" rx="4" fill="#1b1b1b"/><path d="M8 3l4.5 2.6v4.8L8 13l-4.5-2.6V5.6z" fill="none" stroke="#fff" stroke-width="1.2" stroke-linejoin="round"/><path d="M3.5 5.6 8 8.2l4.5-2.6M8 8.2V13" fill="none" stroke="#fff" stroke-width="1.2" stroke-linejoin="round"/>',
  ),
  // A command run with `agent-graph run`: a play button.
  run: svgImage('<rect width="16" height="16" rx="4" fill="#6b7280"/><path d="M6.2 4.6v6.8L11.6 8z" fill="#fff"/>'),
};

/** Another app's mark: the first letter of its name. */
const letterIcon = (provider) =>
  svgImage(
    `<rect width="16" height="16" rx="4" fill="#6b7280"/><text x="8" y="11.6" text-anchor="middle" font-family="system-ui,sans-serif" font-size="10" font-weight="700" fill="#fff">${
      (appName(provider)[0] || '?').toUpperCase().replace(/[^A-Z0-9]/, '?')
    }</text>`,
  );

/** The icon of the app a session ran in, named for screen readers and on hover. */
function appIcon(provider) {
  const name = appName(provider);
  return h('img', { class: 'app-icon', src: APP_ICON[provider] || letterIcon(provider), alt: name, title: name, width: 16, height: 16 });
}

function nodeName(node) {
  if (!node) return 'Unknown';
  if (node.kind === 'session') {
    // After its folder, so sessions in one folder, or named alike, are told apart.
    if (node.title) {
      const dir = basename(node.cwd);
      return dir && dir !== node.title ? `${dir}: ${node.title}` : node.title;
    }
    // Untitled (a headless run, say): what the session that started it said it was for.
    if (node.purpose) return node.purpose;
    // One session started by another often shares its folder, so say which.
    if (node.parent) return `${PROVIDER_NAME[node.provider] || node.provider} session ${short(node.id)}`;
    return basename(node.cwd) || `Session ${short(node.id)}`;
  }
  // Its name, where its agent gives it one (Codex's nicknames), else its id.
  // Codex's "default" type says nothing, so a named one is just its name.
  if (node.title && (!node.agent_type || node.agent_type === 'default')) return node.title;
  return `${node.agent_type || 'Agent'} ${node.title || short(node.id)}`;
}

/** Node `id` of `graph`: of its tree, or (only what names it) one the tree refers to. */
function nodeOf(graph, id) {
  return graph && (graph.nodes[id] || (graph.others && graph.others[id]));
}

function nameOf(graph, id) {
  const node = nodeOf(graph, id);
  return node ? nodeName(node) : short(id);
}

function clock(ts) {
  const d = new Date(ts);
  const time = d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
  const today = new Date(nowMs()).toDateString() === d.toDateString();
  return today ? time : `${d.toLocaleDateString([], { month: 'short', day: 'numeric' })} ${time}`;
}

function ago(ts) {
  const s = Math.max(0, (nowMs() - new Date(ts).getTime()) / 1000);
  if (s < 10) return 'just now';
  if (s < 60) return `${Math.floor(s)}s ago`;
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86400)}d ago`;
}

function plural(n, word) {
  return `${n} ${word}${n === 1 ? '' : 's'}`;
}

// ---------- data flow ----------

let refreshing = null;
let refreshAgain = false;

function scheduleRefresh() {
  if (refreshing) {
    refreshAgain = true;
    return;
  }
  refreshing = refresh()
    .then(() => setError(null))
    .catch((e) =>
      setError(
        e.needsKey
          ? 'Open this page with the link `agent-graph view` printed: it carries the key for this run.'
          : `Can't reach the viewer: ${e.message}. Is \`agent-graph view\` still running?`,
      ),
    )
    .finally(() => {
      refreshing = null;
      if (refreshAgain) {
        refreshAgain = false;
        scheduleRefresh();
      }
    });
}

/**
 * Looks again at `ms` (the graph's clock), when the graph will change with
 * nothing new happening: a session that has gone quiet in the middle of a
 * command is then shown as probably asking for approval.
 */
let recheckTimer = null;
function recheckAt(ms) {
  clearTimeout(recheckTimer);
  recheckTimer = ms ? setTimeout(scheduleRefresh, Math.max(0, ms - nowMs()) + 250) : null;
}

async function refresh() {
  // A new map: a step still loading lands in the old one.
  S.cache = new Map();
  const before = S.root;
  const asked = S.root || hashId();
  let live;
  try {
    live = await source.graph(null, asked);
  } catch (e) {
    if (S.root !== before) return; // no longer shown: nothing to say
    throw e;
  }
  // Another session was chosen meanwhile; the refresh that follows shows it.
  if (S.root !== before) return;
  S.live = live;
  recheckAt(live.recheck_ms);

  const back = S.returnTo;
  S.returnTo = null;
  if (live.root !== S.root && back) {
    // The address named a node that doesn't exist: back to the one it was on
    // (if that has gone too, the refresh that follows says so).
    history.replaceState(history.state, '', `#${encodeURIComponent(back)}`);
    switchTo(back);
    return;
  }
  if (live.root !== S.root) {
    // The session asked for has gone, or none was yet: the one the reply
    // holds instead (the newest), if it's one to show. Nothing of the old
    // one's (not its timeline to step through either).
    forgetLoads();
    const tree = live.root;
    S.root = tree && (tree === asked || visibleRoots().includes(tree)) ? tree : null;
    // The list hides that one (it's another app's, say): the newest it shows.
    const listed = !S.root && tree ? visibleRoots()[0] : null;
    if (listed) {
      S.root = S.selected = listed;
      S.following = true;
      S.shown = S.live;
      S.stops = [];
      S.pos = -1;
      scheduleRefresh();
      renderAll();
      return;
    }
    S.selected = S.root;
    S.following = true;
    S.shown = S.live;
    S.stops = [];
    S.pos = -1;
  }
  // The timeline of the tree the reply holds, which is now the one shown.
  const stops = S.root ? live.stops : [];

  // Where the timeline is now, after the request: it may have been moved.
  const old = S.stops;
  const currentId = old[S.pos] && old[S.pos].id;
  S.stops = stops;
  let gone = false;
  if (S.following || !currentId || !stops.length) {
    if (!S.following) forgetLoads();
    S.following = true;
    S.pos = S.stops.length - 1;
    S.shown = S.live;
  } else {
    // Stay on the same event while new ones arrive; if it has left the
    // timeline, go back to the nearest one before it that's still there.
    const at = new Map(stops.map((s, i) => [s.id, i]));
    let i = at.get(currentId) ?? -1;
    gone = i < 0;
    for (let k = S.pos - 1; i < 0 && k >= 0; k--) i = at.get(old[k].id) ?? -1;
    S.pos = Math.max(0, i);
  }
  renderAll();
  if (gone) goTo(S.pos);
}

let fetchTimer = null;

/** Keeps any step still loading from being shown: the view has moved on. */
function forgetLoads() {
  clearTimeout(fetchTimer);
  S.seq++;
}

/** Moves the timeline to stop `pos`. The last stop means "live". */
function goTo(pos) {
  if (!S.stops.length) return;
  S.pos = clamp(pos, 0, S.stops.length - 1);
  S.following = S.pos === S.stops.length - 1;
  renderTimeline();
  renderMode();
  forgetLoads();
  if (S.following) {
    S.shown = S.live;
    renderView();
    return;
  }
  const id = S.stops[S.pos].id;
  const cached = S.cache.get(id);
  if (cached) {
    remember(S.cache, id, cached);
    S.shown = cached;
    renderView();
    return;
  }
  // Coalesce requests while the slider is being dragged.
  const seq = S.seq;
  const cache = S.cache;
  fetchTimer = setTimeout(async () => {
    try {
      const graph = await source.graph(id, S.root);
      remember(cache, id, graph);
      if (seq !== S.seq) return;
      S.shown = graph;
      renderView();
    } catch (e) {
      if (seq === S.seq) setError(`Couldn't load that step: ${e.message}`);
    }
  }, 40);
}

/**
 * Keeps step `id`'s graph in `cache` as the most recently shown, forgetting
 * the least recently shown past `CACHED_STEPS`: a long timeline's graphs
 * would otherwise pile up while the page stays open.
 */
function remember(cache, id, graph) {
  cache.delete(id);
  cache.set(id, graph);
  while (cache.size > CACHED_STEPS) cache.delete(cache.keys().next().value);
}

function goLive() {
  goTo(S.stops.length - 1);
}

/** Shows the tree under `id`, if the live graph has it (it may have gone since the list was drawn). */
function selectRoot(id) {
  if (!known(id)) return;
  const open = () => {
    switchTo(id);
    openPage();
  };
  // Opening a session's page: its name moves from the list to the heading.
  if (NARROW.matches && !paged()) {
    const item = [...document.querySelectorAll('.session')].find((b) => b.dataset.id === id);
    transition(open, item && item.querySelector('.s-name'), headingName);
  } else open();
}

const headingName = () => document.querySelector('#view .h-name');
const listedName = () => document.querySelector('.session.selected .s-name');

/**
 * Runs `update` as a view transition (where the browser has them, and motion
 * isn't reduced): the page fades from before to after, and the session's
 * name moves from `from` (an element now) to where `to()` (after) is.
 */
function transition(update, from, to) {
  const still = matchMedia('(prefers-reduced-motion: reduce)').matches;
  if (!document.startViewTransition || still || !from) {
    update();
    return;
  }
  from.style.viewTransitionName = 'session-name';
  const t = document.startViewTransition(() => {
    from.style.viewTransitionName = '';
    update();
    const target = to();
    if (target) target.style.viewTransitionName = 'session-name';
  });
  // Cut short (another tap, or the page hidden) isn't a failure: the change
  // is made either way. Its name is freed once it's done.
  const done = () => {
    const target = to();
    if (target) target.style.viewTransitionName = '';
  };
  t.updateCallbackDone.catch(() => {});
  t.ready.catch(() => {});
  t.finished.then(done, done);
}

/**
 * A phone-sized window, where the columns stack: a session chosen in the
 * list opens as a page of its own (otherwise its tree and details would be
 * below the whole list, out of sight).
 */
const NARROW = matchMedia('(max-width: 720px)');

/**
 * Whether the shown session is open as a page of its own. It's a history
 * entry (see `openPage`), so the browser's Back closes it, as the page's
 * own back button does.
 */
function paged() {
  return NARROW.matches && Boolean(history.state && history.state.agentGraphPage);
}

/** Opens the shown session as a page of its own, on a narrow screen. */
function openPage() {
  if (!NARROW.matches) return;
  if (!paged()) history.pushState({ agentGraphPage: true }, '', location.href);
  renderPage();
  window.scrollTo(0, 0);
}

/** Back to the list: the history entry before the page's, if it made one. */
function closePage() {
  if (history.state && history.state.agentGraphPage) history.back();
  else renderPage();
}

/**
 * On a phone the timeline's a drawer: hidden at first, shown with the
 * Timeline pill, hidden with the arrow atop it. The page leaves room for
 * it while it's up (--timeline-room), and the graph's resized to fit what's
 * left (see `sizeGraph`).
 */
/**
 * The button that shows the timeline on a phone, while it's hidden: beside
 * the graph's legend, or in the cards view, beside the view switch. Its icon
 * is the timeline's slider: a line with the thumb on it.
 */
function timelineButton() {
  const ns = 'http://www.w3.org/2000/svg';
  const svg = document.createElementNS(ns, 'svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('aria-hidden', 'true');
  const line = document.createElementNS(ns, 'line');
  for (const [k, v] of Object.entries({ x1: 3, y1: 12, x2: 21, y2: 12, class: 'track-line' })) line.setAttribute(k, v);
  const thumb = document.createElementNS(ns, 'circle');
  for (const [k, v] of Object.entries({ cx: 15, cy: 12, r: 4.5, class: 'track-thumb' })) thumb.setAttribute(k, v);
  svg.append(line, thumb);
  return h(
    'button',
    { type: 'button', class: 'btn timeline-open', title: 'Show the timeline', 'aria-label': 'Show the timeline', onclick: () => setTimelineOpen(true) },
    svg,
  );
}

function setTimelineOpen(open) {
  S.timelineOpen = open;
  renderTimelineDrawer();
}

function renderTimelineDrawer() {
  const drawer = NARROW.matches;
  const closed = drawer && !S.timelineOpen;
  document.body.classList.toggle('timeline-closed', closed);
  const room = !drawer || closed ? 0 : $('.timeline').offsetHeight + 8;
  document.documentElement.style.setProperty('--timeline-room', `${room}px`);
  sizeGraph();
}

/**
 * Sizes the graph to the room it has on screen: from its top to the
 * bottom of the view, or to the timeline's top where that's over it, so
 * nothing's drawn out of sight (and zoomed out, every other session's in
 * view). It's fitted again as its size changes (see `buildGraph`).
 */
function sizeGraph() {
  const host = document.querySelector('#view .graph-host');
  if (!host || !host.isConnected) return;
  const top = host.getBoundingClientRect().top + (NARROW.matches ? window.scrollY : 0);
  let bottom;
  if (NARROW.matches) {
    // The page scrolls: the window, less the timeline drawer (or its pill).
    const room = parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--timeline-room')) || 0;
    bottom = window.innerHeight - room;
  } else {
    // The middle column scrolls: its bottom, less its padding.
    const main = $('#main');
    bottom = main.getBoundingClientRect().bottom - 16;
  }
  const height = Math.round(Math.max(NARROW.matches ? 300 : 380, bottom - top - 12));
  if (Math.abs(host.offsetHeight - height) > 1) host.style.height = `${height}px`;
}

function renderPage() {
  const on = paged();
  const node = nodePaged();
  document.body.classList.toggle('paged', on);
  document.body.classList.toggle('node-page', node);
  $('#back').hidden = !on;
  // From a node's details, back is to its session; from a session, to the list.
  const root = node && S.live && (S.live.nodes[S.root] || S.live.sessions[S.root]);
  $('#back-label').textContent = node ? (root ? nodeName(root) : 'Back') : 'Sessions';
  $('#page-toggles').hidden = !on || node;
  renderTimelineDrawer();
}

/**
 * Whether a node's details are open as a page of their own, on a narrow
 * screen: a card tapped in a session's page opens them (see `selectNode`),
 * a history entry after the session's, so Back returns to the session.
 */
function nodePaged() {
  return paged() && Boolean(history.state && history.state.agentGraphNode);
}

/** Shows the tree under `id`: live, until its timeline comes. */
function switchTo(id) {
  if (id === S.root) return;
  closeModal();
  // Another session's tree: its page, not the node's details.
  if (nodePaged()) history.replaceState({ agentGraphPage: true }, '', location.href);
  forgetLoads();
  S.root = id;
  S.selected = id;
  S.following = true;
  // Live, until its timeline comes: not a step of the session left.
  S.shown = S.live;
  S.stops = [];
  S.pos = -1;
  // (Keeping the entry's state: whether it's a session's page.)
  history.replaceState(history.state, '', `#${encodeURIComponent(id)}`);
  scheduleRefresh();
  renderAll();
}

function selectNode(id) {
  S.selected = id;
  // One named in the details dialog is shown in it.
  if (S.modal) S.modal = id;
  // On a phone, its details open as a page of their own (one opened from
  // another's details takes that one's place, so Back is to the session).
  if (paged() && !S.modal) {
    const state = { agentGraphPage: true, agentGraphNode: id };
    if (nodePaged()) history.replaceState(state, '', location.href);
    else history.pushState(state, '', location.href);
    renderPage();
    window.scrollTo(0, 0);
  }
  renderView();
}

/**
 * The node named in the address (`#<id>`), whether or not it exists. One
 * that doesn't decode (a stray `%`, say) names nothing.
 */
function hashId() {
  try {
    return decodeURIComponent(location.hash.slice(1)) || null;
  } catch {
    return null;
  }
}

/** Whether `id` is a session in the live graph, or a node it has (of its tree, or one it refers to). */
function known(id) {
  return Boolean(S.live && nodeOf(S.live, id)) || Boolean(S.live && S.live.sessions[id]);
}


function visibleRoots() {
  if (!S.live) return [];
  const cutoff = nowMs() - RECENT_MS;
  const byApp = appFilterOn();
  return S.live.roots.filter((id) => {
    if (id === S.root) return true;
    const root = S.live.sessions[id];
    if (byApp && S.hiddenApps.has(root.provider)) return false;
    if (!S.showDone && isDone(root) && !root.busy) return false;
    return S.showAll || new Date(root.last_event_at).getTime() >= cutoff;
  });
}

/** Finished, and nothing to see: completed or canceled. A failure stays in view. */
function isDone(node) {
  return node.state === 'completed' || node.state === 'canceled';
}

/** Whether the tree leaves `node` out: it's done, and so is everything under it. */
function hidden(graph, node, keep) {
  if (S.showDone || keep.has(node.id) || !isDone(node)) return false;
  return node.children.every((id) => !graph.nodes[id] || hidden(graph, graph.nodes[id], keep));
}

/** The apps the sessions ran in, most sessions first, with how many each. */
function appsInUse() {
  const counts = new Map();
  for (const id of S.live ? S.live.roots : []) {
    const root = S.live.sessions[id];
    if (root) counts.set(root.provider, (counts.get(root.provider) || 0) + 1);
  }
  return [...counts].sort((a, b) => b[1] - a[1] || appName(a[0]).localeCompare(appName(b[0])));
}

/**
 * Whether the list leaves out the apps unticked in the filter. Only while
 * there's more than one app, and so a filter to change it: sessions never
 * vanish for a choice made when there was.
 */
function appFilterOn() {
  const apps = appsInUse();
  // None ticked means all of them: the same as none hidden.
  return S.hiddenApps.size > 0 && apps.length > 1 && !apps.every(([app]) => S.hiddenApps.has(app));
}

/** No app ticked means every app: then none are hidden (and all show as ticked). */
function noneMeansAll() {
  if (appsInUse().every(([app]) => S.hiddenApps.has(app))) S.hiddenApps = new Set();
}

function savedCollapsed() {
  try {
    const saved = JSON.parse(localStorage.getItem('agentGraphCollapsedFolders') || '[]');
    return new Set(Array.isArray(saved) ? saved.filter((f) => typeof f === 'string') : []);
  } catch {
    return new Set();
  }
}

/** Folds folder `cwd`'s sessions away in the list, or opens them again. */
function toggleFolder(cwd) {
  if (!S.collapsed.delete(cwd)) S.collapsed.add(cwd);
  try {
    localStorage.setItem('agentGraphCollapsedFolders', JSON.stringify([...S.collapsed]));
  } catch {
    // Not remembered, then; it still applies now.
  }
  renderSessions();
}

function savedHiddenApps() {
  try {
    const saved = JSON.parse(localStorage.getItem('agentGraphHiddenApps') || '[]');
    return new Set(Array.isArray(saved) ? saved.filter((a) => typeof a === 'string') : []);
  } catch {
    return new Set();
  }
}

function saveHiddenApps() {
  try {
    localStorage.setItem('agentGraphHiddenApps', JSON.stringify([...S.hiddenApps]));
  } catch {
    // Not remembered, then; it still applies now.
  }
}

/**
 * The Apps dropdown above the list: a checkbox for each app the sessions
 * ran in, and one for all of them. Shown only when there's more than one.
 * Built again only when the apps change, so it stays open, and keeps focus,
 * as events arrive.
 */
function renderAppFilter() {
  const box = $('#app-filter');
  const apps = appsInUse();
  box.hidden = apps.length < 2;
  if (box.hidden) {
    box.open = false;
    return;
  }
  const menu = box.querySelector('.app-menu');
  const key = apps.map(([app]) => app).join('\n');
  if (box.dataset.apps !== key) {
    box.dataset.apps = key;
    const all = h('input', { type: 'checkbox', 'data-app': '' });
    all.addEventListener('change', () => {
      S.hiddenApps = all.checked ? new Set() : new Set(appsInUse().map(([app]) => app));
      noneMeansAll();
      appsChanged();
    });
    menu.replaceChildren(
      h('label', null, all, h('span', null, 'All apps')),
      h('hr'),
      ...apps.map(([app]) => {
        const tick = h('input', { type: 'checkbox', 'data-app': app });
        tick.addEventListener('change', () => {
          if (tick.checked) S.hiddenApps.delete(app);
          else S.hiddenApps.add(app);
          noneMeansAll();
          appsChanged();
        });
        return h('label', null, tick, appIcon(app), h('span', null, appName(app)), h('span', { class: 'count' }, ''));
      }),
    );
  }
  const hidden = apps.filter(([app]) => S.hiddenApps.has(app));
  // None ticked is all of them (see `noneMeansAll`).
  const shown = hidden.length === apps.length ? apps : apps.filter(([app]) => !S.hiddenApps.has(app));
  for (const tick of menu.querySelectorAll('input[data-app]')) {
    const app = tick.dataset.app;
    if (app) {
      tick.checked = shown.some(([a]) => a === app);
      const count = apps.find(([a]) => a === app);
      tick.parentElement.querySelector('.count').textContent = count ? String(count[1]) : '';
    } else {
      tick.checked = shown.length === apps.length;
      tick.indeterminate = shown.length > 0 && shown.length < apps.length;
    }
  }
  $('#app-filter-label').textContent =
    shown.length === apps.length
      ? 'All'
      : shown.length <= 2
          ? shown.map(([app]) => appName(app)).join(', ')
          : `${shown.length} of ${apps.length}`;
}

/**
 * Hangs the Apps menu directly below its button: from the button's left
 * edge, or, if that would run off the window (the button's at the end of a
 * row), to its right edge.
 */
function placeAppMenu(box) {
  const menu = box.querySelector('.app-menu');
  const button = box.querySelector('summary').getBoundingClientRect();
  const width = menu.getBoundingClientRect().width;
  const page = document.documentElement.clientWidth;
  // From its left edge, or, without room for that, to its right edge; on the page either way.
  let left = button.left;
  if (left + width > page - 8) left = button.right - width;
  menu.style.left = `${Math.max(8, Math.min(left, page - width - 8))}px`;
  menu.style.top = `${button.bottom + 6}px`;
}

function appsChanged() {
  saveHiddenApps();
  renderSessions();
}

function savedShowDone() {
  try {
    return localStorage.getItem('agentGraphShowDone') !== 'false';
  } catch {
    return true;
  }
}

function setError(message) {
  S.error = message;
  const banner = $('#banner');
  banner.hidden = !message;
  banner.textContent = message || '';
}

// ---------- rendering ----------

function renderAll() {
  renderPage();
  renderMode();
  renderSessions();
  renderView();
  renderTimeline();
}

function renderView() {
  renderMain();
  renderDetail();
  renderModal();
}

function renderMode() {
  const mode = $('#mode');
  let pill;
  if (!S.connected) pill = h('span', { class: 'pill offline' }, 'Reconnecting…');
  else if (S.following) pill = h('span', { class: 'pill live' }, source.liveLabel);
  else {
    const stop = S.stops[S.pos];
    pill = h('span', { class: 'pill past' }, `Viewing ${stop ? clock(stop.ts) : 'the past'}`);
  }
  mode.replaceChildren(pill);
}

function renderSessions() {
  renderAppFilter();
  redraw($('#session-list'), sessionItems());
  renderAttention();
}

/**
 * The listed sessions by the folder they ran in (`cwd`, '' where none was
 * recorded): each folder where its newest session would be, with its
 * sessions newest first, and named by its last part, or its last two where
 * another listed folder ends the same.
 */
function folderGroups() {
  const groups = new Map();
  for (const id of visibleRoots()) {
    const cwd = (S.live.sessions[id] && S.live.sessions[id].cwd) || '';
    if (!groups.has(cwd)) groups.set(cwd, []);
    groups.get(cwd).push(id);
  }
  const parts = (cwd) => cwd.split(/[\\/]/).filter(Boolean);
  const last = new Map();
  for (const cwd of groups.keys()) {
    const name = parts(cwd).pop() || '';
    last.set(name, (last.get(name) || 0) + 1);
  }
  return [...groups].map(([cwd, ids]) => {
    const p = parts(cwd);
    const name = !cwd ? 'No folder' : last.get(p[p.length - 1]) > 1 ? p.slice(-2).join('/') : p[p.length - 1];
    return { cwd, name, ids };
  });
}

/** The listed sessions that need you, in the list's order, folded away or not. */
function attentionIds() {
  return folderGroups()
    .flatMap((g) => g.ids)
    .filter((id) => S.live.sessions[id] && S.live.sessions[id].needs_you);
}

/**
 * The counter above the list: how many sessions need you, with arrows that
 * step through them. Hidden when none do.
 */
function renderAttention() {
  const box = $('#attention');
  const ids = attentionIds();
  box.hidden = !ids.length;
  if (!ids.length) {
    S.attentionAt = null;
    redraw(box, []);
    return;
  }
  const at = ids.indexOf(S.attentionAt);
  const n = ids.length;
  const label = at >= 0 ? `${at + 1} of ${n} need you` : n === 1 ? '1 session needs you' : `${n} sessions need you`;
  const arrow = (dir, d, name) =>
    h(
      'button',
      { class: 'icon-btn', 'aria-label': name, title: name, onclick: () => stepAttention(dir) },
      svgIcon(d),
    );
  redraw(box, [
    h('span', { class: 'attention-count', 'aria-live': 'polite' }, label),
    arrow(-1, 'M3 10l5-5 5 5', 'Previous session that needs you'),
    arrow(1, 'M3 6l5 5 5-5', 'Next session that needs you'),
  ]);
}

/**
 * Scrolls the list to the next (`dir` 1) or previous (-1) session that needs
 * you, centering it, and wrapping around at either end. Beside the list (not
 * on a phone, where the list is a page of its own), it also shows that
 * session, with what in it needs you brought into view and pulsing (see
 * `showNeedsYou`).
 */
function stepAttention(dir) {
  const ids = attentionIds();
  if (!ids.length) return;
  let at = ids.indexOf(S.attentionAt);
  at = at < 0 ? (dir > 0 ? 0 : ids.length - 1) : (at + dir + ids.length) % ids.length;
  S.attentionAt = ids[at];
  // Its folder's opened, if it's folded away.
  const cwd = S.live.sessions[ids[at]].cwd || '';
  if (S.collapsed.has(cwd)) toggleFolder(cwd);
  renderSessions();
  const item = [...document.querySelectorAll('#session-list .session')].find((e) => e.dataset.id === ids[at]);
  if (!item) return;
  const still = matchMedia('(prefers-reduced-motion: reduce)').matches;
  item.scrollIntoView({ block: 'center', behavior: still ? 'auto' : 'smooth' });
  // Highlighted each time it's stepped to, even if it was last time.
  item.classList.remove('spotlight');
  void item.offsetWidth;
  item.classList.add('spotlight');
  if (!NARROW.matches) {
    S.showNeedsYou = ids[at];
    if (ids[at] !== S.root) selectRoot(ids[at]);
    else renderMain();
  }
}

/**
 * On a screen wide enough for the details pane at the right, shows node
 * `id`'s there (the node stepped to with the attention arrows, see
 * `stepAttention`), and marks it selected in the cards or graph drawn now.
 * Narrower, the node's only brought into view: its details would be out of
 * the way (see `showDetails`).
 */
function detailNeedsYou(id) {
  if (!WIDE.matches || S.selected === id) return;
  S.selected = id;
  renderDetail();
  for (const el of document.querySelectorAll('#view .node[data-id]')) el.classList.toggle('selected', el.dataset.id === id);
  if (window.d3 && G.svg) G.svg.selectAll('g.gnode').classed('selected', (n) => n.id === id);
}

/**
 * The node under `rootId` that needs you: the first waiting for you, from
 * the top, or the session itself if none of its nodes says so.
 */
function needsYouIn(graph, rootId) {
  const queue = [rootId];
  const seen = new Set();
  while (queue.length) {
    const id = queue.shift();
    const n = graph.nodes[id];
    if (!n || seen.has(id)) continue;
    seen.add(id);
    if (n.state === 'input_required') return id;
    queue.push(...n.children);
  }
  return rootId;
}

/** How long something brought to your attention pulses. */
const PULSE_MS = 2600;

/** Makes `el` pulse for a moment (again, if it was). */
function pulse(el) {
  el.classList.remove('attention');
  void el.getBoundingClientRect();
  el.classList.add('attention');
  clearTimeout(el.attentionTimer);
  el.attentionTimer = setTimeout(() => el.classList.remove('attention'), PULSE_MS);
}

/** A 16×16 stroked icon; SVG needs its own namespace, which `h` doesn't give. */
function svgIcon(d) {
  const ns = 'http://www.w3.org/2000/svg';
  const svg = document.createElementNS(ns, 'svg');
  svg.setAttribute('viewBox', '0 0 16 16');
  svg.setAttribute('aria-hidden', 'true');
  const path = document.createElementNS(ns, 'path');
  path.setAttribute('d', d);
  svg.append(path);
  return svg;
}

/**
 * The list: a heading for each folder, which folds its sessions away or
 * opens them again, and its sessions under it, named without the folder
 * (see `listName`).
 */
function sessionItems() {
  if (!S.live) return [];
  const groups = folderGroups();
  if (!groups.length) {
    const count = S.live.roots.length;
    if (count && appFilterOn()) return [h('li', { class: 'empty-note' }, 'No sessions to show from the apps chosen.')];
    const what = S.showDone ? 'older session' : 'older or completed session';
    return [h('li', { class: 'empty-note' }, count ? `${plural(count, what)} hidden.` : 'No sessions yet.')];
  }
  return groups.map(({ cwd, name, ids }) => {
    const open = !S.collapsed.has(cwd);
    const needing = ids.filter((id) => S.live.sessions[id].needs_you).length;
    return h(
      'li',
      { class: 'folder' },
      h(
        'button',
        {
          class: 'folder-head',
          'data-id': cwd,
          'aria-expanded': String(open),
          title: cwd || 'Sessions with no folder recorded',
          onclick: () => toggleFolder(cwd),
        },
        svgIcon('M6 4l4 4-4 4'),
        h('span', { class: 'folder-name' }, name),
        // Folded away, a session in it that needs you still shows.
        !open && needing
          ? h('span', { class: 'folder-attention', title: needing === 1 ? 'A session needs you' : `${needing} sessions need you` })
          : null,
        h('span', { class: 'folder-count', 'aria-label': plural(ids.length, 'session') }, String(ids.length)),
      ),
      open ? h('ul', { class: 'folder-sessions' }, ids.map(sessionItem)) : null,
    );
  });
}

/** A session in the list, under its folder: its name there, and its state. */
function sessionItem(id) {
  // What's going on in the session's tree, worked out where the graph is.
  const root = S.live.sessions[id];
  const { agents, needs_you: needsYou, deadlocked, stuck, busy } = root;
  const dotState = needsYou ? 'input_required' : busy && root.state === 'idle' ? 'working' : root.state;
  const done = root.tasks - root.open_tasks;
  // (On the site's /watch, sharing from several computers, which it's on.)
  const meta = [root.host || null, root.provider, agents ? plural(agents, 'agent') : null, root.tasks ? `tasks ${done}/${root.tasks}` : null]
    .filter(Boolean)
    .join(' · ');
  const name = listName(root);
  return h(
    'li',
    null,
    h(
      'button',
      {
        // `spotlight`: the one the attention counter stepped to (see `stepAttention`).
        class: `session${id === S.root ? ' selected' : ''}${needsYou ? ' needs-you' : ''}${needsYou && id === S.attentionAt ? ' spotlight' : ''}`,
        'data-id': id,
        'aria-current': id === S.root ? 'true' : null,
        onclick: () => selectRoot(id),
      },
      h('span', { class: `dot ${dotState}` }),
      // The whole name on hover, where the list has cut it short.
      h('span', { class: 's-title', title: name }, appIcon(root.provider), h('span', { class: 's-name' }, name)),
      h('span', { class: 's-when' }, ago(root.last_event_at)),
      needsYou
        ? h('span', { class: 's-sub attention' }, `Needs you: ${needsYou.attention || nodeName(needsYou)}`)
        : deadlocked
          ? h('span', { class: 's-sub problem' }, 'Deadlocked: waiting on a session that waits on it')
          : stuck
            ? h('span', { class: 's-sub problem' }, `Looks stuck: ${stuck.id === id ? 'no activity' : nodeName(stuck)}`)
            : h('span', { class: 's-sub' }, root.headline || STATE_LABEL[root.state]),
      h('span', { class: 's-meta' }, meta),
    ),
  );
}

/**
 * A session's name in the list, under its folder's heading: `nodeName`
 * without the folder it puts first. One with no title, and nothing else to
 * go by, is named by its id, as its folder's already said.
 */
function listName(node) {
  if (node.title) return node.title;
  if (node.purpose || node.parent) return nodeName(node);
  return `Session ${short(node.id)}`;
}

function renderMain() {
  const view = $('#view');
  const { kids, graph, ringed, flash, keep } = mainView();
  redraw(view, kids);
  const host = view.querySelector('.graph-host');
  if (host && graph) {
    sizeGraph();
    renderGraph(host, graph, keep, ringed, flash);
    return;
  }
  // (Kept while a session chosen in it comes: sized as it was.)
  if (host) sizeGraph();
  // Not drawing the graph: its simulation stops, and starts afresh when it is.
  if (G.sim) {
    G.sim.stop();
    G.host = null;
  }
  if (!graph) return;
  if (S.showNeedsYou && S.showNeedsYou === S.root && graph.nodes[S.root]) {
    S.showNeedsYou = null;
    const id = needsYouIn(graph, S.root);
    detailNeedsYou(id);
    const card = [...view.querySelectorAll('.node')].find((e) => e.dataset.id === id);
    if (card) {
      const still = matchMedia('(prefers-reduced-motion: reduce)').matches;
      card.scrollIntoView({ block: 'center', behavior: still ? 'auto' : 'smooth' });
      pulse(card);
      return;
    }
  }
  // A card flashes each time an event touches it, even if it did last time.
  const flashed = flash && view.querySelector('.node.flash');
  if (flashed) {
    flashed.classList.remove('flash');
    void flashed.offsetWidth;
    flashed.classList.add('flash');
  }
  // Brought into view when what it stands for changes (the session, the
  // step, the node it touched); not whenever the tree is drawn again (a card
  // chosen, new events, the step's graph arriving), or it would undo the
  // reader's scrolling. Only a card drawn counts as brought into view: one
  // not in the graph still shown (its step's is on its way) is, once it is.
  const stop = S.stops[S.pos];
  const key = ringed ? `${S.root} ${stop && stop.id} ${ringed}` : null;
  const ring = view.querySelector('.node.current');
  if (!ringed) S.scrolledTo = null;
  else if (ring) {
    if (key !== S.scrolledTo) ring.scrollIntoView({ block: 'nearest' });
    S.scrolledTo = key;
  }
}

/**
 * What the main view shows (`kids`), and when it's a tree, the graph it's
 * from, and the nodes it rings and flashes.
 */
function mainView() {
  if (!S.live) return { kids: [] };

  if (!S.live.roots.length) {
    return {
      kids: [
        h(
          'div',
          { class: 'empty' },
          h('h2', null, 'No sessions recorded yet'),
          h('p', null, 'Install the hooks, then start a new Claude Code session. It will appear here as soon as it starts.'),
          h('pre', null, h('code', null, 'agent-graph install claude-code')),
          S.info && S.info.where ? h('p', null, 'Watching ', h('code', null, S.info.where)) : null,
        ),
      ],
    };
  }
  if (!S.root) {
    return {
      kids: [
        h(
          'div',
          { class: 'empty' },
          h('h2', null, S.showDone ? 'Nothing in the last 24 hours' : 'Nothing running in the last 24 hours'),
          h(
            'p',
            null,
            S.showDone
              ? 'Tick “Older” in the sessions list to see earlier sessions.'
              : 'Tick “Completed” or “Older” in the sessions list to see more sessions.',
          ),
        ),
      ],
    };
  }

  const graph = S.shown || S.live;
  const root = graph.nodes[S.root];
  // Its summary until its tree comes.
  const liveRoot = S.live.nodes[S.root] || S.live.sessions[S.root] || (S.live.others || {})[S.root];
  if (!liveRoot) return { kids: [h('p', { class: 'empty-note' }, 'Loading…')] };
  const stop = S.stops[S.pos];
  // Chosen from the others in the graph, and its tree still to come.
  const arriving = !root && S.view === 'graph' && S.following && G.arrive && G.arrive.id === S.root;
  // Its state: at the step, or while its tree comes, as it is now.
  const state = root ? root.state : arriving ? liveRoot.state : null;

  const head = h(
    'div',
    { class: 'view-head' },
    h(
      'div',
      { class: 'title-row' },
      // Named as it was at the step being viewed: sessions get renamed.
      h('h1', null, h('span', { class: 'h-name' }, nodeName(root || liveRoot)), state ? h('span', { class: `state ${state}` }, STATE_LABEL[state]) : null),
      h(
        'div',
        { class: 'title-tools' },
        root || arriving ? viewSwitch() : null,
        root && S.view === 'cards' ? timelineButton() : null,
        root || arriving ? saveImageLink(stop) : null,
      ),
    ),
    h(
      'div',
      { class: 'meta' },
      h('span', null, liveRoot.provider),
      h('span', { class: 'mono', title: liveRoot.id }, short(liveRoot.id)),
      liveRoot.cwd ? h('span', { class: 'mono', title: liveRoot.cwd }, liveRoot.cwd) : null,
      liveRoot.started_at ? h('span', null, `Started ${clock(liveRoot.started_at)}`) : null,
    ),
    !S.following && stop
      ? h('div', { class: 'past-note' }, `As of ${clock(stop.ts)} — step ${S.pos + 1} of ${S.stops.length}`)
      : null,
  );

  if (!root) {
    // Chosen from the others in the graph, while its tree comes: the graph
    // stays as it is (the one chosen at its middle), not a note in its place.
    if (arriving) {
      return { kids: [head, h('div', { class: 'graph-host', 'data-keep': '' })] };
    }
    const note = S.following ? 'Loading…' : 'This session hadn’t started yet at this point.';
    return { kids: [head, h('p', { class: 'empty-note' }, note)] };
  }

  // Which node to ring: the one the current step touched.
  let ringed = null;
  let flash = null;
  if (!S.following && stop) ringed = stop.node;
  const last = S.stops[S.stops.length - 1];
  if (S.following && last && last.id !== S.lastFlashed) {
    flash = S.lastFlashed ? last.node : null; // don't flash on first load
    S.lastFlashed = last.id;
  }

  // What the reader is looking at stays, done or not.
  const keep = new Set([S.root, S.selected, ringed].filter(Boolean));
  const tree =
    S.view === 'graph'
      ? h('div', { class: 'graph-host', 'data-keep': '' })
      : h('div', { class: 'tree' }, branch(graph, root, ringed, flash, keep));
  const left = Object.values(graph.nodes).filter((n) => n.id !== S.root && isDone(n) && hidden(graph, n, keep)).length;
  const what = left === 1 ? 'completed agent or session' : 'completed agents and sessions';
  const note = left ? h('p', { class: 'empty-note' }, `${left} ${what} hidden.`) : null;
  return { kids: note ? [head, tree, note] : [head, tree], graph, ringed, flash, keep };
}

// ---------- graph view ----------
//
// The tree drawn as a network instead of cards, with D3: each session and
// agent a node, laid out top down by d3-force (a level for each generation),
// lines from each to the ones it started, and dashed arrows from one that's
// waiting to what it's waiting on. The simulation and its nodes outlive each
// redraw, so as the timeline moves or the "Completed" filter changes, nodes
// move to their new places, new ones grow out of their parents, and gone
// ones shrink away. Hovering a node shows a summary; clicking it, its full
// details in a dialog (see `openModal`).

/** Where D3 is served from: beside this script (the site serves both from /viewer/). */
const ASSET_BASE = (() => {
  try {
    return new URL('.', document.currentScript.src).href;
  } catch {
    return new URL('/', location.href).href;
  }
})();

/**
 * The tasks pill's check mark: assets/images/check-mark.png at 32px, built
 * in, so neither viewer serves another file for it.
 */
const CHECK_MARK = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAACAAAAAgCAYAAABzenr0AAACu0lEQVR42r2Xu2tUQRTGf/dms0o04GO7gJUGMUJIIT7QQhALwcZCbGyCVbBIoxgbUcH8BZIilZWdjYIQsFAsLUR8ISpoYxfXYNhkH3dsvoHjMHfv3c1uBoaZOzt7vnO+85iZhO1rKZBo7oBsu4ATYKTbj8O22ls6CdT0vQ58GDYLI2a8DzREvQNWgfGQgKTPHmsVjQeAFwLtyOLfwDnD0MCbBz8N/BT4psDXgFN54KPAGLCzhz5mWEgM+GWBOqAp6xvAWYP1X9sDvAX+AHWNRX0V+AssSUZV43WTZm11B1yKgXuNx4Fp4CbwWeuugGonq78r0JrALWDRRLeTrHngicBbMWEToujwFny/IMC2FGjpeykwljwFNoETsqZq0sjPY32H9swLrGWod8BLszcpo8CxoEDtNrmc5ET7bADu0+2X5JZKN6/AcbP2CPgBXIwUFw9+wVicGRc44HwZ6mMMjAKPTdVywENgVxDt08prb7Uzfr/XawBNKIpnRPuGLGkZ4e+AM9pfA74FFvvxtSivAbeNG0vFwEl9Xwn82jLldAF4GoB6FtaAQ5Ixo98my8RBGIQAywFIJ3BLZuZewWvG2ikVqoP9KFCRzz8F4D7grDJewWdBpTsqV3ZVIO1yT1gHrhrrnLlc+P9lWqsDc5o7erwwxFpHLLwB7gi0E9mXScYNnXxpv5eM0AWpsTYBXgV02/lKUCPSQbiAIOBmFVBJ4I6GTj96pZ6SJTKTZV9Fs6fYU/8A+KI9W7rfxVwQu9+tmKz4qMMoDYrMwFwQO/vnpGiqE3Czn8iPnWh5F1OrgHfFXeCI2KiIjSRHRk8KuKDkhq2tcTGyFqYwRlYpBbzG+4C9Ws8KHjOuS2C3gf2R+MhVoCmNn2uelnhuuYLsqWrcKBLk25QYcDCwN2EdeD9AmcN5ucaez4Nqhc/wf/6J9Aaq2G0YAAAAAElFTkSuQmCC';

/** The Fit button's icon: assets/images/zoom.png at 40px, built in too. */
const ZOOM_ICON = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAACgAAAAoCAYAAACM/rhtAAAJjElEQVR42sWYWWxc1RnH/985d5nxzMR2vJDNewKNs5BAMBAIJqhqqY0QhU54CE0kqFpFiEpIiD5V02klhAQIolQ8ICRUaEFiJIpcJYG0CCwRm8Rpm4QwLY3NOAvJeEliz36Xc74+eBwgJLGJo/CXrmbuPNz5ne/8v+UewiUUjUZlIpFQANBx7+Z5bFp3sa8eYGA1NNczsAAEl4A0gUZB9LFpUY810bS/tzfuX/iMKxVd7LdYLEbxeFzf8sDPa+DJX2uttxCJZsMwIMAgAgQRGAytAQaBAXiuo0F0SAixvWtd0xvxeFzPFZIucc/rurf+EhoxaVqLLEmwTck11ZW6rq6OQqEKCkci5Ps+crk8ZzNZHhkb5Ww2Lx2f4SkFrdR+wH/ywK4/980Fki783tnZKQuhppdImo9bUiAStPzW1ma56sYbqaGxGfNra2HbFqQUUyvRGsVCAel0GkNHB3H40GH95ek0Fz0tXddxwGrbgd1/fu1KIen8ZzQqogCGcxXvCNO6P2iQv2TRdeLODRvEqrVrUVVViQoDCEiGbRAIwMlzJTiKYUgJwzBARBhJp/FJXx8GBg6o0TOTwlNMWrtPH9j1xnOdnZ1Gb2+v/10A5bSZk4mECjWt+6Mw7Ucituldv6zFfGhTlNbcfBPmVxhYEBa4LmIgbBsImhIBUyLjaJQ8DWKG7yt4vofwvHlYvmIFwqEKcWY0zYViSStNP17Ytuqz/g96jkSjUZlMJnnWgNOhv7lr62PCMP8QNKV3w7Jm8+HNm7Fg0WK4xSKWLwzBNiQUA5NFH+cKPkayLrIlBUEEEIHKl1YKvuehta0NVVVVdPJYCvliibXW3U0/WPPX9999czwWi4ne3t5ZQcpkMok7u7dVe8p5N2CbwebF14mf/ixKjY0NENqBzwRPMwquwomzJYxmPWRKCo6vp+Au9EwZ1HEcNDQ1QQpBp04cUyVPB1zPbTw9ePit+vp6MdsoCgBcUvknTTtQFwla+o4NG0TrsqWoC2osXxhBdYWB0xMOTk04cHyGIQimIBhEl1+5lCgWCrjtjvVob283LEmKhHH/rd1b704kEioajcpZAXbcu3meZv2YJYlbW5rlqrVrEZYKNWEbzIDrMwxJkIJABDC+umYSM0NIidvuWI/62mq2LAu+4ie+S5IIJusuIY1FAVNg5erVNL+6CrUVU4sbz7nIlHxIIvAV1DAhBDzHQXNLC9paW6WEhmbdeWf3tupyyaGZAVk9ZBgG5ldX6YamFgQNRoUtoTRjJOuWO8aVSzPDMAy0Lm2jgGVq07RqXC7eVa4eYmZA8GoBRl1dHdXU1iBoTJXFrDOdCJiTiAhaayxpaMC8eWEtpGQFtRIARkdHZ44gGDUEIBSqIDtgwy5bN+coMOOqSGuNilAIAdsGAUQaC2bvQWCBEEA4HCFDSshydvpK42qJmWFZFipCIRAABi0CgPr6ep5NmXGZAV/5YOav+Y1wNaW1hla6/FjtzTqCIKQ1A/lcnn1fwddTiLYxt+S4MJtd10WhkJ+2zclZe5CAMYCQzWa5UCigVG7lYVtCiqsDSUIgMzmJYqkEZkCQPD3rxRHoYwUgPTrGYyMjKPgAwAjZEoagOScKM0MQ4VgqhUwmJ33fg5Bi36w9KIV8x3Ndnc3mRGpwEEWfMTxexPGzJfhqanqei6SUKBaLGBoc0o5i0sr/Ihts3A8AiURixkwU/hLzX8T6iKMYhw4d1mPpNCYcYCzrzhlOaw3LtvGfI58hNTysFROEEDuTibhb7sUzR/Cfr7ziGVI85/mKTqXT+pO+PlimATnHCs3lDpLNZNDf18fnMnnhuaW8xYHtAJBob5/lNBOLicZQ8S3lugcLrjYGBg6off39qAiFpsrCFcIREYQQ2LN7N75IDSsFKQTwct97rwxFo1GJeFzPbqKurxfJREI1LFt7yFdqi+N64sxoGlVVlbSksQGe553/w9luq2EYkFLiH3v2YO/efn0mU5ClUmFfwLZ+cdvqpSoBAMkoATMPrRLJJEejUfn3njdPLFy6+gSTfLBQLOmTx1IQQlJTcxNMy4Lypwr5ZbNVCASCQRTyeezs6cHevX16ZHxCCSlla8Piz5fVtL3x2msveEgmGehljkHEey/vQwkAyWSSOzs7jf4Pev69oG1FUWn8KF8s0akTx/2R06dFOBzC/Joa2LYNIcT5qVkIAcMwYJomTNOE67o4fPAgdvb8jQ9+ekSfzRSklFIurqv251dXL8352du3Pvx4AkGxsFh/S+VTrx+cfDsalYnLTNd0sdOEW7q2bNJMr1qWFbEkdH3tfG5rbRWtS9uooaEBwVAIlmlCM8NzXWQmJ5FKpfDF4JBODQ/rc5m8oSBRzOf2X9/SkKusqronn895oVDYPDM28lHy+PiKprASf1nx6X21z3z6yYexTmNj/OJve3SpI4913Y+sJMg4Mz1oWRYkGEHb0PMiEW3bdjmJFAqFPIrFEjK5vHR9TYoJrlPKSkO+rDD2+99s2eL3fHxkZzBQ8cOz4+PjQ6fP1Aa06736aMlYYZyd9D4a6rZ2HO/jWKdBF4GkGc9lurZsZMhfKa06DcNcMLXFmKqRXB7/GVC+B631URL0PrG5Y2D3q/+bft4TT2y3T+WP7RpKT2yw4Tp/6jjKNzRxCKtaBQbTEziQ6qLnj/VfDPLSqRmLTU275XJwZ/e2ag/OBs3cztpfzITFYHYBnJRCfElCDlT74YH33tvhnF9kezsj/ju+vWtz4znH2F8ZoMjrHZ/719cUIyoHUKWtRUerwNH0BPpTXbT925Az1o7pt6/ZHltMRb+dgbgGYoIR53u6Hm00TWfgpZtS85ZX5yzlGSQFAE8BkYBCR7PE0ZGLRvK7tAuKRqNiekSabvRfvy/31m9kJMdiguJx7b5wc4dpqT3QshK+UiCSIAI8vwzZMgV5QSSv7lR6Cb39dlRu2pRQ7rPr1psVeieAKnhag2jK0J6CjthadLQIDI6cQ1+q+zwkrpGmI8LPr1kPS+4E+AJIH4gEy9s9OoGB1L30wvC+awb4LUiTdoGoEp7+art9HwgHFW5tkfjvqXHs//Luawr4Dchn16xH8DzkNyNZWaGwrlFi3/BH1xwQAPjDToM2Tm+3mPbkVCQFAQUXWNPIOucUxfcBSBt7fY51GvTUwT6U1H1gnoQlJZgVNAMBU6MyAFFyJ74XQACgeBny6UN74eAnYB5D0JDQDIQtgYk8YTTzDOF71nlPPrd6JSzjRWi+FQLjyHgv0m+P7Pg/V5v3jVVoNicAAAAASUVORK5CYII=';

/** How far apart the generations are, top to bottom. */
const LEVEL = 110;

const G = {
  host: null, // the element the graph is drawn in (kept by `morph`: data-keep)
  svg: null,
  sim: null,
  byId: new Map(), // id -> the simulation's node, kept across redraws (its position)
  userMoved: false, // zoomed or panned by the reader: no more fitting to the view
  key: null, // the tree drawn last (root), to start afresh on another
  loading: null,
  overview: false, // zoomed out to show the other sessions around this one
  switching: false, // zooming into another session, to show it
  following: null, // the node the view's following (see `focusNode`)
};

function savedViewMode() {
  // A link can choose it for its page (the site's examples do), leaving the
  // reader's own choice as it was.
  const asked = new URLSearchParams(location.search).get('view');
  if (asked === 'graph' || asked === 'cards') return asked;
  try {
    return localStorage.getItem('agentGraphView') === 'graph' ? 'graph' : 'cards';
  } catch {
    return 'cards';
  }
}

function setViewMode(mode) {
  if (S.view === mode) return;
  S.view = mode;
  try {
    localStorage.setItem('agentGraphView', mode);
  } catch {
    // Not remembered, then.
  }
  renderMain();
}

/** The Cards / Graph switch, in the view's heading. */
function viewSwitch() {
  const option = (mode, label) =>
    h(
      'button',
      {
        type: 'button',
        class: `view-option${S.view === mode ? ' on' : ''}`,
        'aria-pressed': String(S.view === mode),
        onclick: () => setViewMode(mode),
      },
      label,
    );
  return h('div', { class: 'view-switch', role: 'group', 'aria-label': 'View as' }, option('cards', 'Cards'), option('graph', 'Graph'));
}

/** Loads D3 once, for the graph view: it isn't needed until that's chosen. */
function loadD3() {
  if (window.d3) return Promise.resolve(window.d3);
  if (!G.loading) {
    G.loading = new Promise((resolve, reject) => {
      const script = document.createElement('script');
      script.src = new URL('d3.min.js', ASSET_BASE).href;
      script.onload = () => (window.d3 ? resolve(window.d3) : reject(new Error('D3 didn’t load')));
      script.onerror = () => {
        G.loading = null;
        script.remove();
        reject(new Error('D3 didn’t load'));
      };
      document.head.append(script);
    });
  }
  return G.loading;
}

/**
 * What the graph shows of `graph`, under `rootId`: the nodes the cards
 * would (the same "Completed" filter, and the same `keep`), each with its
 * generation, and the lines between them: `tree` from each node to the
 * ones it started, and `wait` from one that's waiting to what it's
 * waiting on, where both are shown.
 */
function graphData(graph, rootId, keep) {
  const root = graph.nodes[rootId];
  if (!root) return { nodes: [], links: [] };
  const nodes = [];
  const shown = new Set();
  const walk = (node, depth, parent) => {
    shown.add(node.id);
    nodes.push({ id: node.id, node, depth, parent });
    for (const id of node.children) {
      const kid = graph.nodes[id];
      if (kid && !shown.has(kid.id) && !hidden(graph, kid, keep)) walk(kid, depth + 1, node.id);
    }
  };
  walk(root, 0, null);
  const links = nodes.filter((d) => d.parent).map((d) => ({ id: `${d.parent}>${d.id}`, source: d.parent, target: d.id, kind: 'tree' }));
  // A wait between a node and its parent or child is along its tree line
  // already (that line shows it, as waiting, by its dash): only others get
  // an arrow of their own.
  const joined = new Set(links.map((l) => `${l.source} ${l.target}`));
  const waits = new Set();
  for (const d of nodes) {
    for (const on of (d.node.blocked && d.node.blocked.on) || []) {
      if (!shown.has(on)) continue;
      if (joined.has(`${d.id} ${on}`) || joined.has(`${on} ${d.id}`)) waits.add(`${d.id} ${on}`);
      else links.push({ id: `${d.id}~${on}`, source: d.id, target: on, kind: 'wait' });
    }
  }
  for (const l of links) if (waits.has(`${l.source} ${l.target}`) || waits.has(`${l.target} ${l.source}`)) l.waiting = true;
  return { nodes, links };
}

/** What shape and size a node is drawn as: a session square, an agent circle, a command diamond. */
function nodeShape(n) {
  if (n.provider === 'run') return { shape: 'command', r: 11 };
  if (n.kind === 'session') return { shape: 'session', r: n.parent ? 12 : 16 };
  return { shape: 'agent', r: 10 };
}

/** The node's outline, as an SVG path around its centre. */
function shapePath(shape, r) {
  if (shape === 'agent') return `M${-r},0a${r},${r} 0 1,0 ${2 * r},0a${r},${r} 0 1,0 ${-2 * r},0`;
  if (shape === 'command') return `M0,${-r * 1.25}L${r * 1.25},0L0,${r * 1.25}L${-r * 1.25},0Z`;
  const c = r * 0.35;
  return `M${-r + c},${-r}H${r - c}Q${r},${-r} ${r},${-r + c}V${r - c}Q${r},${r} ${r - c},${r}H${-r + c}Q${-r},${r} ${-r},${r - c}V${-r + c}Q${-r},${-r} ${-r + c},${-r}Z`;
}

/** A pill `w` wide and `h` high, around its centre: a tasks item's outline. */
function pillPath(w, h) {
  const r = h / 2;
  const x = w / 2 - r;
  return `M${-x},${-r}H${x}A${r},${r} 0 0,1 ${x},${r}H${-x}A${r},${r} 0 0,1 ${-x},${-r}Z`;
}

/** The most of a name shown under a node, in characters. */
const LABEL_MAX = 16;
/** The most of words shown under a node (what it's doing): a little more. */
const WORDS_MAX = 24;

/**
 * A name short enough to sit under a node (the whole one's in its summary
 * and hover tip). An agent's is what it was started for ("Port refunds"),
 * which tells it from its siblings better than its type and id do. A
 * session's is its title, after its folder only if that isn't `home`, the
 * folder of the session the page is showing (named, with it, at the top):
 * a tree's sessions are nearly always in that one. Those are cut at a word.
 * Any other keeps its start and its end, with an ellipsis between, since
 * names often differ only at the end ("worker-1", "worker-2").
 */
function graphLabel(n, home) {
  const dir = n.kind === 'session' && basename(n.cwd);
  const elsewhere = dir && dir !== home && dir !== n.title;
  const said = n.kind === 'agent' ? n.purpose : n.title && (elsewhere ? `${dir}: ${n.title}` : n.title);
  const words = said ? said.replace(/\s+/g, ' ').trim() : '';
  if (words) {
    if (words.length <= WORDS_MAX) return words;
    // A space just after the room left for the ellipsis still ends a word that fits.
    const space = words.slice(0, WORDS_MAX).lastIndexOf(' ');
    return `${(space > WORDS_MAX / 2 ? words.slice(0, space) : words.slice(0, WORDS_MAX - 1)).trimEnd()}…`;
  }
  const name = nodeName(n);
  if (name.length <= LABEL_MAX) return name;
  const tail = Math.floor((LABEL_MAX - 1) / 2.5);
  return `${name.slice(0, LABEL_MAX - 1 - tail).trimEnd()}…${name.slice(-tail).trimStart()}`;
}

/**
 * Draws (or updates) the graph of `graph`'s tree in `host`. `ringed` is the
 * node the current step touched; `flash`, one a new event just did.
 */
function renderGraph(host, graph, keep, ringed, flash) {
  if (!window.d3) {
    if (!host.firstChild) host.append(h('p', { class: 'graph-note' }, 'Loading the graph…'));
    loadD3().then(
      () => renderMain(),
      (e) => {
        host.replaceChildren(h('p', { class: 'graph-note' }, 'The graph view couldn’t load. The cards view still works.'));
        setError(`Couldn't load the graph view: ${e.message}`);
      },
    );
    return;
  }
  const d3 = window.d3;
  const still = matchMedia('(prefers-reduced-motion: reduce)').matches;
  const ms = still ? 0 : 450;

  // A new host (the view switched, or the page redrawn) starts afresh; so
  // does another session's tree.
  const fresh = G.host !== host || G.key !== S.root;
  // Chosen from the others (see `goToSession`): it comes from where it was.
  const arrive = fresh && !still && G.arrive && G.arrive.id === S.root ? G.arrive : null;
  if (fresh) {
    // (Nothing's arriving under the copy of what was chosen: it goes.)
    if (!arrive) dropGhost();
    G.arrive = null;
    if (G.arriving) G.arriving.stop();
    G.arriving = null;
    if (G.sim) G.sim.stop();
    G.host = host;
    G.key = S.root;
    G.byId = new Map();
    G.userMoved = false;
    G.overview = false;
    G.switching = false;
    G.placed = null;
    G.lastK = null;
    host.replaceChildren();
    buildGraph(d3, host);
  }

  const { nodes: data, links } = graphData(graph, S.root, keep);
  const home = basename((nodeOf(graph, S.root) || {}).cwd);
  // The same object for a node from one drawing to the next, so it keeps
  // its place; a new one starts at its parent's, and grows out of it.
  const before = new Set(G.byId.keys());
  const nodes = data.map((d) => {
    let n = G.byId.get(d.id);
    if (!n) {
      const from = d.parent && G.byId.get(d.parent);
      n = {
        id: d.id,
        x: from ? from.x + (Math.random() - 0.5) * 30 : (Math.random() - 0.5) * 10,
        y: from ? from.y + 20 : d.depth * LEVEL,
      };
      G.byId.set(d.id, n);
    }
    Object.assign(n, { node: d.node, depth: d.depth, parent: d.parent, label: graphLabel(d.node, home) }, nodeShape(d.node));
    // Its tasks, as one pill beside it: how many are done, of how many.
    const total = d.node.tasks.length;
    n.tasks = total ? `${total - d.node.open_tasks}/${total}` : null;
    // Its check mark, then its count.
    n.pillW = n.tasks ? n.tasks.length * 6.6 + 30 : 0;
    // Room for its name beside its neighbours' (about 6.5px a character),
    // and its pill.
    n.room = Math.max(n.r + 22, n.label.length * 3.3 + 10, n.tasks ? n.r + 10 + n.pillW : 0);
    return n;
  });
  const rowsBefore = new Map(nodes.map((n) => [n.id, n.ty]));
  arrangeRows(nodes);
  const moved = nodes.some((n) => rowsBefore.get(n.id) !== undefined && rowsBefore.get(n.id) !== n.ty);
  const now = new Set(nodes.map((n) => n.id));
  for (const id of G.byId.keys()) if (!now.has(id)) G.byId.delete(id);
  const changed = nodes.length !== before.size || nodes.some((n) => !before.has(n.id));

  const root = d3.select(host).select('g.viewport');

  // Lines, under the nodes.
  root
    .select('g.links')
    .selectAll('path.link')
    .data(links, (l) => l.id)
    .join(
      (enter) => enter.append('path').attr('opacity', 0).call((e) => e.transition().duration(ms).attr('opacity', 1)),
      (update) => update,
      (exit) => exit.transition().duration(ms).attr('opacity', 0).remove(),
    )
    .attr('class', (l) => `link ${l.kind}${l.waiting ? ' waiting' : ''}`);

  // Nodes: each a group, moved by the simulation, holding a group scaled
  // as it comes and goes.
  root
    .select('g.nodes')
    .selectAll('g.gnode')
    .data(nodes, (n) => n.id)
    .join(
      (enter) => {
        const g = enter
          .append('g')
          .attr('class', 'gnode')
          .attr('tabindex', 0)
          .attr('role', 'button')
          .attr('transform', (n) => `translate(${n.x},${n.y})`)
          .on('click', (e, n) => showDetails(n.id, onPill(e) ? 'Tasks' : null))
          .on('keydown', (e, n) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault();
              showDetails(n.id, onPill(e) ? 'Tasks' : null);
            }
          })
          .on('pointerenter pointermove', (e, n) => showGraphTip(e, n))
          .on('pointerleave', hideGraphTip)
          .on('focus', (e, n) => showGraphTip(e, n))
          .on('blur', hideGraphTip)
          .call(dragNodes(d3));
        const inner = g.append('g').attr('class', 'grow').attr('transform', 'scale(0.01)');
        inner.append('circle').attr('class', 'halo');
        inner.append('path').attr('class', 'shape');
        // A session's app (Claude Code, Codex …), in its square, so sessions
        // of different apps are told apart; its state still shows round it.
        inner.append('image').attr('class', 'gapp');
        inner.append('text').attr('class', 'glabel');
        // Its tasks' pill (shown when it has some): its own stop for the keyboard.
        const pill = inner.append('g').attr('class', 'pill').attr('tabindex', 0).attr('role', 'button');
        pill.append('path');
        pill.append('image').attr('href', CHECK_MARK).attr('width', 10).attr('height', 10).attr('y', -5);
        pill.append('text').attr('y', 3.5);
        pill.on('focus', (e, n) => showGraphTip(e, n)).on('blur', hideGraphTip);
        inner.transition().duration(ms).ease(d3.easeBackOut).attrTween('transform', () => scaleFrom(0.01, 1));
        return g;
      },
      (update) => update,
      (exit) =>
        exit
          .classed('leaving', true)
          .on('click keydown pointerenter pointermove pointerleave focus blur .drag', null)
          .call((e) => e.select('g.grow').transition().duration(ms).attrTween('transform', () => scaleFrom(1, 0.01)))
          .transition()
          .duration(ms)
          .attr('opacity', 0)
          .remove(),
    )
    .attr('class', (n) =>
      [
        'gnode',
        `k-${n.shape}`,
        n.node.state,
        n.id === S.selected ? 'selected' : '',
        n.id === ringed ? 'current' : '',
        n.node.stale ? 'stale' : '',
      ]
        .filter(Boolean)
        .join(' '),
    )
    .attr('aria-label', (n) => `${nodeName(n.node)}, ${STATE_LABEL[n.node.state]}`)
    .each(function (n) {
      const g = d3.select(this);
      g.select('circle.halo').attr('r', n.r + 7);
      g.select('path.shape').attr('d', shapePath(n.shape, n.r));
      const app = n.shape === 'session' ? APP_ICON[n.node.provider] || letterIcon(n.node.provider) : null;
      const size = Math.round(n.r * 1.25);
      g.select('image.gapp')
        .attr('display', app ? null : 'none')
        .attr('href', app)
        .attr('width', size)
        .attr('height', size)
        .attr('x', -size / 2)
        .attr('y', -size / 2);
      g.select('text.glabel')
        .attr('y', n.r + 15)
        .text(n.label);
      const total = n.node.tasks.length;
      const done = total - n.node.open_tasks;
      g.select('g.pill')
        .attr('display', n.tasks ? null : 'none')
        .attr('tabindex', n.tasks ? 0 : null)
        .attr('aria-label', n.tasks ? `Tasks: ${done} of ${total} done` : null)
        .classed('all-done', Boolean(total) && done === total)
        .attr('transform', `translate(${n.r + 8 + n.pillW / 2},0)`);
      g.select('g.pill path').attr('d', n.tasks ? pillPath(n.pillW, 18) : null);
      g.select('g.pill image').attr('x', -n.pillW / 2 + 7);
      g.select('g.pill text')
        .attr('x', 7)
        .text(n.tasks || '');
    });

  // A node an event just touched flashes (again, if it did last time).
  if (flash) {
    const g = root.selectAll('g.gnode').filter((n) => n.id === flash);
    g.classed('flash', false);
    void host.offsetWidth;
    g.classed('flash', true);
  }

  if (S.showNeedsYou && S.showNeedsYou === S.root && graph.nodes[S.root]) {
    S.showNeedsYou = null;
    G.focus = needsYouIn(graph, S.root);
    detailNeedsYou(G.focus);
  }

  const sim = G.sim;
  sim.nodes(nodes);
  sim.force('link').links(links);
  // Once it's settled, it stays still: it moves when something changes.
  sim.alphaTarget(0);
  if (still) {
    sim.stop();
    for (let i = 0; i < 300; i++) sim.tick();
    tick();
    fitGraph(false);
  } else if (arrive) {
    arriveFrom(arrive, nodes);
  } else {
    // Woken only when its shape changes: nodes come or go, or rows move.
    // Otherwise (a node chosen, its state changed) it stays as it is.
    if (changed) sim.alpha(0.7).restart();
    else if (moved) sim.alpha(Math.max(sim.alpha(), 0.35)).restart();
  }
  if (G.focus && !G.arriving) focusNode();
  drawOthers();
  // A link can ask for the other sessions to be shown (?zoom=out): once,
  // when the graph's first drawn and has settled a little.
  if (!G.zoomedOutOnce && new URLSearchParams(location.search).get('zoom') === 'out') {
    G.zoomedOutOnce = true;
    setTimeout(showOthers, still ? 0 : 1200);
  }
}

/**
 * Brings node `G.focus` to the middle of the graph (keeping its zoom), and
 * makes it pulse. While it pulses, the view follows it as the layout
 * settles around it, gliding, until the reader pans or zooms themselves.
 */
function focusNode() {
  const id = G.focus;
  G.focus = null;
  if (!window.d3 || !G.svg || !G.byId.has(id)) return;
  G.host.scrollIntoView({ block: 'nearest' });
  // Where the reader's been shown something, the graph stays put for them.
  G.userMoved = true;
  G.following = id;
  const end = performance.now() + PULSE_MS + 400;
  const still = matchMedia('(prefers-reduced-motion: reduce)').matches;
  let last = performance.now();
  const step = () => {
    const d3 = window.d3;
    const n = G.byId.get(id);
    if (G.following !== id || !G.svg || !n) return;
    const time = performance.now();
    const done = time >= end;
    const box = G.svg.node().getBoundingClientRect();
    if (box.width && box.height) {
      const now = d3.zoomTransform(G.svg.node());
      const k = Math.max(now.k, 0.8);
      const x = box.width / 2 - k * n.x;
      const y = box.height / 2 - k * n.y;
      // A glide, not a jump: most of the way in about a third of a second,
      // however often frames come; exactly there at the end.
      const a = still || done ? 1 : 1 - Math.exp(-(time - last) / 110);
      G.svg.call(G.zoom.transform, d3.zoomIdentity.translate(now.x + (x - now.x) * a, now.y + (y - now.y) * a).scale(now.k + (k - now.k) * a));
    }
    last = time;
    if (done) G.following = null;
    else requestAnimationFrame(step);
  };
  step();
  const g = G.svg.selectAll('g.gnode').filter((d) => d.id === id).node();
  if (g) pulse(g);
}

/**
 * A session chosen from the others arrives: laid out first, where it'll be,
 * then its node glides from where it was chosen (`a`, on screen) to its
 * place, as large as it was shrinking to its own size; then its children
 * spread down out of it, and theirs out of them, each generation a moment
 * after the one before.
 */
function arriveFrom(a, nodes) {
  const d3 = window.d3;
  const sim = G.sim;
  const top = G.byId.get(S.root);
  const box = G.svg.node().getBoundingClientRect();
  if (!top || !box.width || !box.height) {
    sim.alpha(0.7).restart();
    return;
  }
  // Where everything ends up, and the view that fits it.
  sim.stop();
  for (let i = 0; i < 300; i++) sim.tick();
  G.userMoved = false;
  fitGraph(false);
  const t = d3.zoomTransform(G.svg.node());
  const end = new Map(nodes.map((n) => [n, { x: n.x, y: n.y }]));
  const [sx, sy] = t.invert([a.x - box.left, a.y - box.top]);
  const MOVE = 600;
  const STEP = 140;
  const SPREAD = 650;
  const deepest = Math.max(0, ...nodes.map((n) => n.depth));
  const total = MOVE + Math.max(0, deepest - 1) * STEP + (deepest ? SPREAD : 0);
  // When each generation's under way: the session's at once, its children's
  // once it's there, and each after them a step later.
  const startOf = (n) => (n === top ? 0 : MOVE + (n.depth - 1) * STEP);
  const lengthOf = (n) => (n === top ? MOVE : SPREAD);

  const g = G.svg.selectAll('g.gnode');
  g.select('g.grow')
    .interrupt()
    .attr('transform', (n) => (n === top ? `scale(${a.r / (top.r * t.k)})` : 'scale(0.01)'))
    .transition()
    .delay(startOf)
    .duration(lengthOf)
    .ease(d3.easeCubicInOut)
    .attrTween('transform', (n) => (n === top ? scaleFrom(a.r / (top.r * t.k), 1) : scaleFrom(0.01, 1)));
  // Its colours turn from those it was chosen in to its own as it goes.
  const shape = g.filter((n) => n === top).select('path.shape');
  const own = shape.node() && getComputedStyle(shape.node());
  if (own && a.fill && (a.fill !== own.fill || a.stroke !== own.stroke)) {
    const [fill, stroke] = [own.fill, own.stroke];
    shape
      // (Its own easing of colour changes would lag behind this one.)
      .style('transition', 'none')
      .style('fill', a.fill)
      .style('stroke', a.stroke)
      .transition('colour')
      .duration(MOVE)
      .ease(d3.easeCubicInOut)
      .style('fill', fill)
      .style('stroke', stroke)
      .on('end interrupt', function () {
        this.style.fill = '';
        this.style.stroke = '';
        this.style.transition = '';
      });
  }
  // Lines come in with what they lead to.
  G.svg
    .selectAll('path.link')
    .interrupt()
    .attr('opacity', 0)
    .transition()
    .delay((l) => (typeof l.target === 'object' ? startOf(l.target) : MOVE))
    .duration(SPREAD / 2)
    .attr('opacity', 1);

  const ease = d3.easeCubicInOut;
  const progress = (n, ms) => ease(Math.max(0, Math.min(1, (ms - startOf(n)) / lengthOf(n))));
  const place = (ms) => {
    // Depth first, so each node's parent is placed before it.
    for (const n of nodes) {
      const e = end.get(n);
      const p = progress(n, ms);
      if (n === top) {
        n.x = sx + (e.x - sx) * p;
        n.y = sy + (e.y - sy) * p;
        continue;
      }
      // Out of its parent, as that moves, by more and more of how far it
      // ends up from it.
      const parent = (n.parent && G.byId.get(n.parent)) || top;
      const pe = end.get(parent) || e;
      n.x = parent.x + (e.x - pe.x) * p;
      n.y = parent.y + (e.y - pe.y) * p;
    }
    tick();
  };
  place(0);
  // Its node's under the copy of what was chosen now: that fades away.
  dropGhost();
  G.arriving = d3.timer((ms) => {
    const done = ms >= total;
    place(done ? total : ms);
    if (!done) return;
    G.arriving.stop();
    G.arriving = null;
    for (const n of nodes) Object.assign(n, end.get(n), { vx: 0, vy: 0 });
    tick();
    if (G.focus) focusNode();
  });
}

/** A transition's `transform`, from scale `a` to `b` (said outright, not parsed from the element). */
function scaleFrom(a, b) {
  return (t) => `scale(${a + (b - a) * t})`;
}

/** How far apart a generation's rows are, when it has more than one. */
const SUBROW = 54;

/**
 * Where each node sits: across (`n.tx`) and down (`n.ty`), laid out as a
 * tidy tree, whose shape the simulation keeps to (see `columnPull`).
 *
 * Across, each subtree gets a band as wide as it needs (its own name, or its
 * children's bands side by side, whichever's wider), and each child's band
 * sits in its parent's, in order: so families stand apart, rather than
 * crowding round the middle.
 *
 * Down, each generation is a band; a family with many children spreads
 * them over a few rows in its band, taking turns, so neighbours sit in
 * different rows and close up across (a family over `r` rows takes about
 * `r` times less width). The root's own children keep to one row, unless
 * there are more than four, and even then take more only where that makes
 * the tree drawn much larger: the top of the tree is where spreading out
 * pays off most. How many rows a family may take is chosen so the tree can be
 * drawn as large as its space allows: a tall phone gets more than a wide
 * window, and a small tree keeps to one.
 */
function arrangeRows(nodes) {
  const ids = new Set(nodes.map((n) => n.id));
  const kids = new Map();
  const roots = [];
  for (const n of nodes) {
    if (n.parent && ids.has(n.parent)) {
      if (!kids.has(n.parent)) kids.set(n.parent, []);
      kids.get(n.parent).push(n);
    } else roots.push(n);
  }
  const box = G.svg && G.svg.node().getBoundingClientRect();
  const W = box && box.width ? box.width : 900;
  const H = box && box.height ? box.height : 560;
  // How many rows a family of `count` at `depth` takes: the root's children
  // one, unless there are more than four, then at most `top` (and a row for
  // every two); others at most `r` (and one each).
  const rowsFor = (count, depth, r, top) =>
    depth === 0 || (depth === 1 && count <= 4)
      ? 1
      : depth === 1
        ? Math.max(1, Math.min(top, Math.ceil(count / 2)))
        : Math.max(1, Math.min(r, count));

  // The tree's width and each subtree's, and each generation's rows, at
  // `r` rows a family (`top` for the root's children).
  const measure = (r, top) => {
    const width = new Map();
    const bandRows = [];
    const band = (depth, rows) => (bandRows[depth] = Math.max(bandRows[depth] || 1, rows));
    const of = (n) => {
      const children = kids.get(n.id) || [];
      const rows = rowsFor(children.length, n.depth + 1, r, top);
      if (children.length) band(n.depth + 1, rows);
      const across = children.reduce((sum, k) => sum + of(k), 0) / rows;
      const w = Math.max(2 * n.room + SUBTREE_GAP, across);
      width.set(n.id, w);
      return w;
    };
    const total = roots.reduce((sum, n) => sum + of(n), 0);
    band(0, 1);
    const height = bandRows.reduce((sum, rows) => sum + LEVEL + ((rows || 1) - 1) * SUBROW, 0);
    return { r, top, width, total, height, bandRows };
  };

  // The rows that let it be drawn largest (and no more, since more only
  // make it taller), where each more row of the root's children has to
  // make it a fifth larger: keeping them in one row, apart, is worth more.
  const scaleOf = (m) => Math.min(1.4, W / (m.total + 80), H / (m.height + 60));
  const worth = (m) => scaleOf(m) * 0.8 ** (m.top - 1);
  let best = measure(1, 1);
  // One row that's drawn about full size already stays one row.
  if (scaleOf(best) < 0.9) {
    const most = Math.max(1, Math.ceil((kids.get(roots[0] && roots[0].id) || []).length / 2));
    for (let top = 1; top <= most; top++) {
      for (let r = 1; r <= 12; r++) {
        const m = measure(r, top);
        if (worth(m) > worth(best) * 1.03) best = m;
      }
    }
  }
  const { r, top: topRows, width, bandRows } = best;
  // Where it's fitted by its width, with height to spare, the generations
  // are spread down it too (up to twice as far apart), rather than leaving
  // it empty.
  const stretch = Math.max(1, Math.min(2, H / (best.height + 60) / (W / (best.total + 80))));

  // Down: each generation's band's top, and each node's row in it.
  const tops = [];
  let top = 0;
  for (let depth = 0; depth < bandRows.length; depth++) {
    tops[depth] = top;
    top += (LEVEL + ((bandRows[depth] || 1) - 1) * SUBROW) * stretch;
  }
  const topOf = (depth) => tops[depth] ?? depth * LEVEL;
  // Across: the children's bands, side by side, spread over the parent's.
  // (`branch`: which of the root's children's subtrees each is in, and
  // where that subtree's column is: see `branchesApart`.)
  const place = (n, center, branch) => {
    n.tx = center;
    n.branch = branch || null;
    const children = kids.get(n.id) || [];
    if (!children.length) return;
    const rows = rowsFor(children.length, n.depth + 1, r, topRows);
    const sum = children.reduce((total, k) => total + width.get(k.id), 0);
    const scale = width.get(n.id) / sum;
    let left = center - width.get(n.id) / 2;
    children.forEach((k, i) => {
      k.ty = topOf(k.depth) + (i % rows) * SUBROW * stretch;
      const w = width.get(k.id) * scale;
      const x = left + w / 2;
      place(k, x, branch || (k.depth === 1 ? { id: k.id, x } : null));
      left += w;
    });
  };
  let left = -best.total / 2;
  for (const root of roots) {
    root.ty = topOf(root.depth);
    place(root, left + width.get(root.id) / 2);
    left += width.get(root.id);
  }
}

/** Room left between neighbouring subtrees, across. */
const SUBTREE_GAP = 24;

/**
 * How firmly a node's drawn to its place across (`arrangeRows`): most
 * near the root, where the tree's shape is set, and less further down,
 * where rows and room between names settle it.
 */
function columnPull(n) {
  return [0.5, 0.35, 0.2][n.depth] ?? 0.08;
}

/**
 * Keeps nodes apart by the room they take up, a box as wide as their name
 * and as tall as their shape and name, rather than a circle, so rows can
 * sit close. Each overlap's undone along the shorter way out.
 */
function boxCollide() {
  let nodes = [];
  const force = () => {
    for (let i = 0; i < nodes.length; i++) {
      const a = nodes[i];
      for (let j = i + 1; j < nodes.length; j++) {
        const b = nodes[j];
        const dx = b.x + b.vx - (a.x + a.vx);
        const ox = a.room + b.room - Math.abs(dx);
        if (ox <= 0) continue;
        const dy = b.y + b.vy - (a.y + a.vy);
        const oy = a.r + b.r + 32 - Math.abs(dy);
        if (oy <= 0) continue;
        if (ox < oy) {
          const push = (dx < 0 ? -1 : 1) * ox * 0.35;
          a.vx -= push;
          b.vx += push;
        } else {
          const push = (dy < 0 ? -1 : 1) * oy * 0.35;
          a.vy -= push;
          b.vy += push;
        }
      }
    }
  };
  force.initialize = (n) => (nodes = n);
  return force;
}

/** How far above or below each other nodes of different subtrees still push apart. */
const BRANCH_REACH = 140;
/** How much room, across, beyond their names, nodes of different subtrees keep. */
const BRANCH_GAP = 48;

/**
 * Keeps the root's children's subtrees apart: nodes of different ones that
 * come close across (nearer than their names and `BRANCH_GAP`) push apart,
 * each toward its own subtree's side (so subtrees never pass through each
 * other), most on the same level, less the further apart they are down,
 * and not at all past `BRANCH_REACH`.
 */
function branchesApart(alpha) {
  const nodes = G.sim ? G.sim.nodes() : [];
  for (let i = 0; i < nodes.length; i++) {
    const a = nodes[i];
    if (!a.branch) continue;
    for (let j = i + 1; j < nodes.length; j++) {
      const b = nodes[j];
      if (!b.branch || b.branch.id === a.branch.id) continue;
      const dy = Math.abs(b.y - a.y);
      if (dy >= BRANCH_REACH) continue;
      const want = a.room + b.room + BRANCH_GAP;
      const dx = b.x - a.x;
      if (Math.abs(dx) >= want) continue;
      // Apart the way their subtrees' columns are, whichever way they are now.
      const side = b.branch.x >= a.branch.x ? 1 : -1;
      const gap = side * dx;
      const push = (want - gap) * 0.12 * alpha * (1 - dy / BRANCH_REACH);
      a.vx -= side * push;
      b.vx += side * push;
    }
  }
}

/** Each node drawn toward its parent, across, so a family stays under it. */
function family(alpha) {
  for (const n of G.sim ? G.sim.nodes() : []) {
    const parent = n.parent && G.byId.get(n.parent);
    if (parent) n.vx += (parent.x - n.x) * 0.1 * alpha;
  }
}

/** Sets up the SVG, its zoom and the simulation, once for each host. */
function buildGraph(d3, host) {
  const svg = d3.select(host).append('svg').attr('class', 'graph').attr('role', 'img').attr('aria-label', 'The session’s agents, as a network');
  const defs = svg.append('defs');
  defs
    .append('marker')
    .attr('id', 'wait-arrow')
    .attr('viewBox', '0 -5 10 10')
    .attr('refX', 10)
    .attr('markerWidth', 7)
    .attr('markerHeight', 7)
    .attr('orient', 'auto')
    .append('path')
    .attr('class', 'wait-head')
    .attr('d', 'M0,-5L10,0L0,5');
  const viewport = svg.append('g').attr('class', 'viewport');
  // The other sessions (see `placeOthers`): zoomed and panned with the rest.
  viewport.append('g').attr('class', 'others');
  viewport.append('g').attr('class', 'links');
  viewport.append('g').attr('class', 'nodes');
  G.svg = svg;
  // Fitted again whenever its space changes size (shown at last, turned, the
  // timeline shown or hidden), unless the reader's moved it themselves.
  if (window.ResizeObserver) {
    let last = '';
    new ResizeObserver(([entry]) => {
      const { width, height } = entry.contentRect;
      const size = `${Math.round(width)}x${Math.round(height)}`;
      if (size === last || !width || !height) return;
      last = size;
      if (!G.userMoved) fitGraph(false);
    }).observe(svg.node());
  }

  G.zoom = d3
    .zoom()
    // Far enough out for the other sessions (see `showOthers`).
    .scaleExtent([0.1, 3])
    .on('zoom', (e) => {
      viewport.attr('transform', e.transform);
      // Once the reader moves it, it stays where they put it (and stops
      // following what it was showing them).
      if (e.sourceEvent) {
        G.userMoved = true;
        G.following = null;
      }
      // Zoomed out far enough, the other sessions show round this one.
      updateOverview(e.transform.k, e.sourceEvent);
    });
  svg.call(G.zoom).on('dblclick.zoom', null);

  const tools = h(
    'div',
    { class: 'graph-tools' },
    h(
      'button',
      { type: 'button', class: 'btn show-others', title: 'All sessions', 'aria-label': 'All sessions', hidden: true, onclick: showOthers },
      'All',
    ),
    h(
      'button',
      { type: 'button', class: 'btn icon-fit', title: 'Fit this session in view', 'aria-label': 'Fit this session in view', onclick: () => fitGraph(true) },
      h('img', { src: ZOOM_ICON, alt: '', width: 20, height: 20 }),
    ),
  );
  const legend = h(
    'ul',
    // On a phone, tapping it hides it again (see `legendButton`).
    { class: 'graph-legend', 'aria-label': 'Legend', onclick: () => setLegendOpen(false) },
    h('li', null, h('span', { class: 'glyph k-session' }), 'Session'),
    h('li', null, h('span', { class: 'glyph k-agent' }), 'Agent'),
    h('li', null, h('span', { class: 'glyph k-command' }), 'Command'),
    h('li', null, h('span', { class: 'glyph k-tasks' }), 'Tasks done'),
    h('li', null, h('span', { class: 'glyph k-wait' }), 'Waiting on'),
  );
  host.classList.toggle('legend-open', S.legendOpen);
  host.append(
    tools,
    h('div', { class: 'graph-foot' }, legendButton(), legend, timelineButton()),
    h('div', { class: 'graph-tip', hidden: true }),
  );

  G.sim = d3
    .forceSimulation()
    .force(
      'link',
      d3
        .forceLink()
        .id((n) => n.id)
        .distance((l) => (l.kind === 'wait' ? 150 : 80))
        .strength((l) => (l.kind === 'wait' ? 0.02 : 0.6)),
    )
    // Pushing apart, less the more there are (boxes keep them from
    // overlapping; more push only spreads a big graph thin).
    .force('charge', d3.forceManyBody().strength(() => -Math.max(60, 320 * Math.min(1, Math.sqrt(12 / Math.max(1, G.sim ? G.sim.nodes().length : 1))))).distanceMax(400))
    .force('collide', boxCollide())
    .force('family', family)
    .force('branches', branchesApart)
    // Top down: each generation in its band, on its row (see `arrangeRows`),
    // the tree centred.
    .force('y', d3.forceY((n) => n.ty ?? n.depth * LEVEL).strength(0.9))
    // Across: each in its family's column (see `arrangeRows`).
    .force('x', d3.forceX((n) => n.tx ?? 0).strength(columnPull))
    .alphaDecay(0.03)
    .on('tick', tick)
    .on('end', () => fitGraph(false));
}

/**
 * On a phone the graph's legend is hidden at first, and this button shows
 * it: its icon is the graph's four shapes (a session's square, an agent's
 * circle, a command's diamond, a tasks pill), set round like a game pad's
 * buttons. Tapping the legend hides it again.
 */
function legendButton() {
  const ns = 'http://www.w3.org/2000/svg';
  const svg = document.createElementNS(ns, 'svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('aria-hidden', 'true');
  const shapes = [
    ['rect', { x: 9, y: 1.5, width: 6, height: 6, rx: 1.2, class: 'k-session' }],
    ['circle', { cx: 19.5, cy: 12, r: 3.2, class: 'k-agent' }],
    ['path', { d: 'M12 15.8 15.7 19.5 12 23.2 8.3 19.5Z', class: 'k-command' }],
    ['rect', { x: 0.8, y: 9.8, width: 7.4, height: 4.4, rx: 2.2, class: 'k-tasks' }],
  ];
  for (const [tag, attrs] of shapes) {
    const el = document.createElementNS(ns, tag);
    for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
    svg.append(el);
  }
  return h('button', { type: 'button', class: 'btn legend-show', title: 'Show the legend', 'aria-label': 'Show the legend', onclick: () => setLegendOpen(true) }, svg);
}

function setLegendOpen(open) {
  S.legendOpen = open;
  if (G.host) G.host.classList.toggle('legend-open', open);
}

/** Moves the drawing to the simulation's positions. */
function tick() {
  const d3 = window.d3;
  if (!G.svg || !d3) return;
  const root = G.svg.select('g.viewport');
  root.selectAll('g.gnode:not(.leaving)').attr('transform', (n) => `translate(${n.x},${n.y})`);
  root.selectAll('path.link').attr('d', (l) => {
    const s = l.source;
    const t = l.target;
    if (typeof s !== 'object' || typeof t !== 'object') return null;
    if (l.kind === 'tree') {
      // A soft curve down from parent to child.
      const my = (s.y + t.y) / 2;
      return `M${s.x},${s.y}C${s.x},${my} ${t.x},${my} ${t.x},${t.y}`;
    }
    // A straight arrow, stopping at the edge of what it points to.
    const dx = t.x - s.x;
    const dy = t.y - s.y;
    const len = Math.hypot(dx, dy) || 1;
    const back = t.r + 6;
    return `M${s.x},${s.y}L${t.x - (dx / len) * back},${t.y - (dy / len) * back}`;
  });
  // Kept fitted to its space as it settles, until it has, unless the
  // reader's moved it.
  if (!G.userMoved && G.sim && G.sim.alpha() > 0.03) fitGraph(false);
}

/** Zooms to fit the whole graph in view (smoothly when asked, by the Fit button). */
function fitGraph(smooth) {
  const d3 = window.d3;
  if (!G.svg || !d3 || !G.sim) return;
  if (smooth) {
    G.userMoved = false;
    setOverview(false);
  }
  else if (G.userMoved || G.arriving) return;
  const nodes = G.sim.nodes();
  if (!nodes.length) return;
  const box = G.svg.node().getBoundingClientRect();
  if (!box.width || !box.height) return;
  // Each node with its name under it (as wide as `room` allows for), and a margin.
  const pad = 36;
  const x0 = Math.min(...nodes.map((n) => n.x - n.room)) - pad;
  const x1 = Math.max(...nodes.map((n) => n.x + n.room)) + pad;
  const y0 = Math.min(...nodes.map((n) => n.y - n.r - 10)) - pad;
  const y1 = Math.max(...nodes.map((n) => n.y + n.r + 22)) + pad;
  const k = Math.min(1.4, box.width / (x1 - x0), box.height / (y1 - y0));
  const t = d3.zoomIdentity.translate(box.width / 2 - (k * (x0 + x1)) / 2, box.height / 2 - (k * (y0 + y1)) / 2).scale(k);
  const still = matchMedia('(prefers-reduced-motion: reduce)').matches;
  if (smooth && !still) G.svg.transition().duration(500).call(G.zoom.transform, t);
  else G.svg.call(G.zoom.transform, t);
}

/**
 * Dragging a node moves it (and the others make room); let go, and it stays
 * where it's put: pinned there (`fx`, `fy`) while the rest settle round it.
 * A click, which doesn't move it, leaves it as it was. Once one's been
 * moved, the view stays put too, rather than being fitted again round it.
 */
function dragNodes(d3) {
  return d3
    .drag()
    .on('start', (e, n) => {
      n.dragging = false;
      hideGraphTip();
    })
    .on('drag', (e, n) => {
      if (!n.dragging) {
        n.dragging = true;
        G.userMoved = true;
        G.following = null;
        // (During a drag, d3 counts it among the active ones: so not `!e.active`.)
        G.sim.alphaTarget(0.3).restart();
      }
      n.fx = e.x;
      n.fy = e.y;
    })
    .on('end', (e, n) => {
      if (!n.dragging) return;
      n.dragging = false;
      if (!e.active) G.sim.alphaTarget(0);
    });
}

/** The summary of node `n` beside the pointer (or the node, from the keyboard). */
function showGraphTip(e, n) {
  const tip = G.host && G.host.querySelector('.graph-tip');
  if (!tip) return;
  if (onPill(e) && n.tasks) {
    const total = n.node.tasks.length;
    const done = total - n.node.open_tasks;
    const doing = n.node.tasks.filter((t) => t.status === 'in_progress');
    redraw(tip, [
      h('div', { class: 'tip-head' }, h('strong', null, `Tasks · ${done} of ${total} done`)),
      h('div', { class: 'tip-sub' }, nodeName(n.node)),
      ...doing.slice(0, 3).map((t) => h('div', null, `▸ ${t.active_text || t.text}`)),
      h('div', { class: 'tip-hint' }, 'Click to see them all'),
    ]);
    placeTip(tip, e);
    return;
  }
  const graph = S.shown || S.live;
  const node = n.node;
  const done = node.tasks.length - node.open_tasks;
  const kind =
    node.provider === 'run'
      ? 'Command (agent-graph run)'
      : node.kind === 'session'
        ? node.parent
          ? 'Session it started'
          : 'Session'
        : `${node.agent_type || 'Agent'}${node.background ? ' (background)' : ''}`;
  redraw(tip, [
    h('div', { class: 'tip-head' }, h('span', { class: `dot ${node.state}` }), h('strong', null, nodeName(node))),
    h('div', { class: 'tip-sub' }, `${kind} · ${STATE_LABEL[node.state]}${node.stale ? ' · no events for a while' : ''}`),
    node.purpose ? h('div', null, node.purpose) : null,
    node.state === 'input_required'
      ? h('div', { class: 'tip-attention' }, `Needs you: ${node.attention || 'waiting for input'}`)
      : node.headline
        ? h('div', { class: 'tip-muted' }, node.headline)
        : null,
    node.blocked && graph ? h('div', { class: 'tip-muted' }, blockedText(graph, node.blocked)) : null,
    node.tasks.length ? h('div', { class: 'tip-muted' }, `${done} of ${node.tasks.length} tasks done`) : null,
    h('div', { class: 'tip-hint' }, 'Click for everything about it'),
  ].filter(Boolean));
  placeTip(tip, e);
}

/** Shows `tip` beside the pointer, or the node when it's focused from the keyboard; kept inside the graph. */
function placeTip(tip, e) {
  tip.hidden = false;
  const area = G.host.getBoundingClientRect();
  const item = itemBox(e && e.currentTarget);
  const w = tip.offsetWidth;
  const hgt = tip.offsetHeight;
  const gap = 10;
  const edge = 8;
  if (!item) {
    tip.style.left = `${edge}px`;
    tip.style.top = `${edge}px`;
    return;
  }
  // The item, in the graph's own coordinates.
  const box = { x0: item.left - area.left, y0: item.top - area.top, x1: item.right - area.left, y1: item.bottom - area.top };
  const clamp = (v, lo, hi) => Math.max(lo, Math.min(v, Math.max(lo, hi)));
  const midX = clamp((box.x0 + box.x1) / 2 - w / 2, edge, area.width - w - edge);
  const midY = clamp((box.y0 + box.y1) / 2 - hgt / 2, edge, area.height - hgt - edge);
  // Beside it, never over it: to its right, left, below or above, the
  // first where the tip fits in the graph; failing all, where the least of
  // it is out of the graph.
  const spots = [
    { x: box.x1 + gap, y: midY },
    { x: box.x0 - gap - w, y: midY },
    { x: midX, y: box.y1 + gap },
    { x: midX, y: box.y0 - gap - hgt },
  ];
  const outside = (p) =>
    Math.max(0, edge - p.x) + Math.max(0, p.x + w - (area.width - edge)) + Math.max(0, edge - p.y) + Math.max(0, p.y + hgt - (area.height - edge));
  const best = spots.find((p) => outside(p) === 0) || spots.reduce((a, b) => (outside(b) < outside(a) ? b : a));
  tip.style.left = `${best.x}px`;
  tip.style.top = `${best.y}px`;
}

/**
 * The part of a graph item the tip mustn't cover, on screen: its shape, its
 * name and its tasks' pill (not the whole cell an other session can be
 * pointed at in).
 */
function itemBox(el) {
  if (!el || !el.getBoundingClientRect) return null;
  const parts = [...el.querySelectorAll('path.shape, text, g.pill:not([display="none"])')].map((p) => p.getBoundingClientRect()).filter((r) => r.width || r.height);
  if (!parts.length) return el.getBoundingClientRect();
  return {
    left: Math.min(...parts.map((r) => r.left)),
    top: Math.min(...parts.map((r) => r.top)),
    right: Math.max(...parts.map((r) => r.right)),
    bottom: Math.max(...parts.map((r) => r.bottom)),
  };
}

function hideGraphTip() {
  const tip = G.host && G.host.querySelector('.graph-tip');
  if (tip) tip.hidden = true;
}

// ---------- the other sessions, zoomed out ----------
//
// Zoomed out far enough, the graph shows the other sessions (those the list
// shows) in a ring around this one, each coloured by how it's doing, and
// pulsing orange where it needs you. Choosing one zooms into it as the rest
// fade, then shows its tree: a way round the sessions without the list,
// which on a phone is a page away.

/** Zoomed out to this, the other sessions fade in… */
const OVERVIEW_IN = 0.36;
/** …and zoomed back in past this, they fade out again (apart, so it doesn't flicker between). */
const OVERVIEW_OUT = 0.5;

/** The other sessions to show around this one: the list's, but for this one. */
function otherSessions() {
  if (!S.live) return [];
  return visibleRoots()
    .filter((id) => id !== S.root && S.live.sessions[id])
    .map((id) => S.live.sessions[id]);
}

/** How a session's doing, as its dot in the list shows it (see `sessionItem`). */
function sessionState(s) {
  return s.needs_you ? 'input_required' : s.busy && s.state === 'idle' ? 'working' : s.state;
}

/** What the list says of a session, in a line (see `sessionItem`). */
function sessionStatus(s) {
  if (s.needs_you) return `Needs you: ${s.needs_you.attention || nodeName(s.needs_you)}`;
  if (s.deadlocked) return 'Deadlocked: waiting on a session that waits on it';
  if (s.stuck) return `Looks stuck: ${s.stuck.id === s.id ? 'no activity' : nodeName(s.stuck)}`;
  return s.headline || STATE_LABEL[s.state];
}

/**
 * Keeps the other sessions in step with the zoom (`source`, the reader's
 * event, if it's theirs): shown once they zoom out far enough, and hidden
 * again once they zoom back in on this session. Zooming in elsewhere (on
 * another session, to read it, say) leaves them be.
 */
function updateOverview(k, source) {
  const before = G.lastK == null ? k : G.lastK;
  G.lastK = k;
  if (G.switching) return;
  if (!G.overview) {
    if (source && k <= OVERVIEW_IN) setOverview(true);
    return;
  }
  if (k >= OVERVIEW_OUT && k > before && (!source || zoomingOnTree(source))) setOverview(false);
}

/** Whether the reader's zoom (`source`: a wheel, or two fingers) centres on this session's tree, as drawn now. */
function zoomingOnTree(source) {
  const box = G.svg.node().getBoundingClientRect();
  const touches = source.touches && source.touches.length ? [...source.touches] : null;
  const at = touches
    ? { x: touches.reduce((a, t) => a + t.clientX, 0) / touches.length, y: touches.reduce((a, t) => a + t.clientY, 0) / touches.length }
    : 'clientX' in source
      ? { x: source.clientX, y: source.clientY }
      : null;
  const nodes = G.sim ? G.sim.nodes() : [];
  if (!at || !nodes.length) return true;
  const t = window.d3.zoomTransform(G.svg.node());
  const [x0, y0] = t.apply([Math.min(...nodes.map((n) => n.x - n.room)), Math.min(...nodes.map((n) => n.y - n.r - 10))]);
  const [x1, y1] = t.apply([Math.max(...nodes.map((n) => n.x + n.room)), Math.max(...nodes.map((n) => n.y + n.r + 22))]);
  const px = at.x - box.left;
  const py = at.y - box.top;
  // A little leeway round it: fingers aren't exact.
  const pad = 40;
  return px >= x0 - pad && px <= x1 + pad && py >= y0 - pad && py <= y1 + pad;
}

// A window of another size: the other sessions, laid out afresh.
window.addEventListener('resize', () => {
  if (!G.overview) return;
  G.placed = null;
  drawOthers();
});

function setOverview(on) {
  if (G.overview === on) return;
  G.overview = on;
  G.placed = null;
  if (G.host) G.host.classList.toggle('overview', on);
  drawOthers();
}

/**
 * Where the other sessions go, on the graph as it's drawn (not in its own
 * units, so they're the same size however far it's zoomed out): a grid over
 * the graph's whole area, its cells as large as leaves one for each session
 * with the tree's part of the view kept clear, and the sessions spread evenly
 * over the cells that are free.
 */
function othersLayout(sessions) {
  if (!sessions.length || !G.svg) return [];
  const box = G.svg.node().getBoundingClientRect();
  const W = box.width || 800;
  const H = box.height || 600;
  // The tree's part of the view, and a margin round it.
  const t = window.d3.zoomTransform(G.svg.node());
  const nodes = G.sim ? G.sim.nodes() : [];
  let clear = { x0: W / 2 - 60, y0: H / 2 - 60, x1: W / 2 + 60, y1: H / 2 + 60 };
  if (nodes.length) {
    const [x0, y0] = t.apply([Math.min(...nodes.map((n) => n.x - n.room)), Math.min(...nodes.map((n) => n.y - n.r - 10))]);
    const [x1, y1] = t.apply([Math.max(...nodes.map((n) => n.x + n.room)), Math.max(...nodes.map((n) => n.y + n.r + 22))]);
    clear = { x0: x0 - 16, y0: y0 - 16, x1: x1 + 16, y1: y1 + 16 };
  }
  const top = 52; // below the graph's buttons
  const bottom = 16; // its legend's hidden meanwhile
  let cells = [];
  let cellW = 0;
  let cellH = 0;
  // The fewest columns (so the largest cells) that leave enough free.
  for (let cols = 2; cols <= 16; cols++) {
    cellW = W / cols;
    const rows = Math.max(1, Math.floor((H - top - bottom) / Math.max(72, cellW * 0.62)));
    cellH = (H - top - bottom) / rows;
    cells = [];
    for (let r = 0; r < rows; r++) {
      for (let c = 0; c < cols; c++) {
        const x = (c + 0.5) * cellW;
        const y = top + (r + 0.5) * cellH;
        const overlaps = x + cellW / 2 > clear.x0 && x - cellW / 2 < clear.x1 && y + cellH / 2 > clear.y0 && y - cellH / 2 < clear.y1;
        if (!overlaps) cells.push({ x, y });
      }
    }
    if (cells.length >= sessions.length) break;
  }
  // Spread evenly over the free cells, not bunched in the first ones.
  return sessions.map((s, i) => {
    const cell = cells[Math.min(cells.length - 1, Math.floor(((i + 0.5) * cells.length) / sessions.length))] || { x: W / 2, y: H / 2 };
    return { id: s.id, session: s, x: cell.x, y: cell.y, w: cellW, h: cellH, i, W, H };
  });
}

/**
 * The other sessions placed in the graph itself, from where `othersLayout`
 * puts them in the view as it's zoomed now: so from then on they zoom and
 * pan with it (to read one, zoom in on it), and are drawn at the size
 * they'd be on screen now (`s`, graph units to a pixel).
 */
function placeOthers(items) {
  const t = window.d3.zoomTransform(G.svg.node());
  return items.map((d) => {
    const [gx, gy] = t.invert([d.x, d.y]);
    const edge = beyondEdge(d);
    const [fx, fy] = t.invert([edge.x, edge.y]);
    return { ...d, gx, gy, fx, fy, s: 1 / t.k };
  });
}

/** Where `d` comes in from, and goes back out to: beyond the graph's edge, straight out from its middle through `d`'s place. */
function beyondEdge(d) {
  const cx = d.W / 2;
  const cy = d.H / 2;
  const len = Math.hypot(d.x - cx, d.y - cy);
  // Straight up, for one right in the middle.
  const [ux, uy] = len ? [(d.x - cx) / len, (d.y - cy) / len] : [0, -1];
  const far = Math.hypot(d.W, d.H) / 2 + 120;
  return { x: cx + ux * far, y: cy + uy * far };
}

/** A transition's `transform`, from where the element is now to (`x`, `y`), at scale `k` (said outright, not parsed from it). */
function moveTo(el, x, y, k = 1) {
  const from = el.agPos || { x, y, k };
  const fk = from.k || 1;
  return (t) => {
    el.agPos = { x: from.x + (x - from.x) * t, y: from.y + (y - from.y) * t, k: fk + (k - fk) * t };
    return `translate(${el.agPos.x},${el.agPos.y}) scale(${el.agPos.k})`;
  };
}

/** Cuts `text` to about `px` wide (at about 6.8px a character). */
function fitText(text, px) {
  const most = Math.max(6, Math.floor(px / 6.8));
  return text.length > most ? `${text.slice(0, most - 1)}…` : text;
}

/**
 * A session's name beside the one the page is showing, whose folder is
 * `home`: just its title when it's in that folder too (it nearly always is,
 * and the top of the page names the folder), else its whole name.
 */
function nameBeside(session, home) {
  return session.title && home && basename(session.cwd) === home ? session.title : nodeName(session);
}

/** Draws (or updates) the other sessions: only while zoomed out, flying in and out. */
function drawOthers() {
  const d3 = window.d3;
  // (Not while one's being gone to: that's all in hand, see `goToSession`.)
  if (!d3 || !G.svg || G.switching) return;
  let items = [];
  if (G.overview) {
    // Where they were put stays (the reader may be reading one); only
    // another set of sessions lays them out afresh.
    const sessions = otherSessions();
    const key = sessions.map((s) => s.id).join('\n');
    if (!G.placed || G.placed.key !== key) G.placed = { key, items: placeOthers(othersLayout(sessions)) };
    const now = new Map(sessions.map((s) => [s.id, s]));
    items = G.placed.items.map((d) => ({ ...d, session: now.get(d.id) || d.session }));
  }
  const ms = matchMedia('(prefers-reduced-motion: reduce)').matches ? 0 : 500;
  const home = basename(((S.live && S.live.sessions[S.root]) || {}).cwd);
  G.svg
    .select('g.others')
    .selectAll('g.onode')
    .data(items, (d) => d.id)
    .join(
      (enter) => {
        const g = enter
          .append('g')
          .attr('class', 'onode')
          .attr('role', 'button')
          .attr('opacity', 0)
          // From beyond the edge…
          .each(function (d) {
            this.agPos = { x: d.fx, y: d.fy, k: d.s };
          })
          .attr('transform', function () {
            return `translate(${this.agPos.x},${this.agPos.y}) scale(${this.agPos.k})`;
          })
          .on('click', (e, d) => goToSession(d))
          .on('keydown', (e, d) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault();
              goToSession(d);
            }
          })
          .on('pointerenter pointermove focus', (e, d) => showOtherTip(e, d))
          .on('pointerleave blur', hideGraphTip);
        const body = g.append('g').attr('class', 'obody');
        // Something to point at across its whole cell, not just its shape.
        body.append('rect').attr('class', 'hit');
        body.append('circle').attr('class', 'halo').attr('r', 22).attr('cy', -12);
        body.append('path').attr('class', 'shape').attr('transform', 'translate(0,-12)').attr('d', shapePath('session', 15));
        body.append('text').attr('class', 'oname').attr('y', 20);
        body.append('text').attr('class', 'ostatus').attr('y', 36);
        // …into place, one after another, overshooting a touch as it settles.
        g.transition()
          .delay((d, i) => (ms ? i * 45 : 0))
          .duration(ms ? 900 : 0)
          .ease(d3.easeBackOut.overshoot(1.1))
          .attr('opacity', 1)
          .attrTween('transform', function (d) {
            return moveTo(this, d.gx, d.gy, d.s);
          });
        return g;
      },
      (update) =>
        update.call((u) =>
          u
            .transition()
            .duration(ms)
            .attr('opacity', 1)
            .attrTween('transform', function (d) {
              return moveTo(this, d.gx, d.gy, d.s);
            }),
        ),
      // Back out the way they came, gathering speed, fading as they go.
      (exit) =>
        exit
          .classed('gone', true)
          .on('click keydown pointerenter pointermove focus pointerleave blur', null)
          .transition()
          .delay((d, i) => (ms ? i * 25 : 0))
          .duration(ms ? 600 : 0)
          .ease(d3.easeBackIn.overshoot(1.3))
          .attr('opacity', 0)
          .attrTween('transform', function (d) {
            // Out from where it is on screen now, past the edge as it's zoomed now.
            const t = d3.zoomTransform(G.svg.node());
            const box = G.svg.node().getBoundingClientRect();
            const pos = this.agPos || { x: d.gx, y: d.gy, k: d.s };
            const [x, y] = t.apply([pos.x, pos.y]);
            const edge = beyondEdge({ x, y, W: box.width || d.W, H: box.height || d.H });
            const [gx, gy] = t.invert([edge.x, edge.y]);
            return moveTo(this, gx, gy, pos.k);
          })
          .remove(),
    )
    .attr('class', (d) => `onode ${sessionState(d.session)}${d.session.needs_you ? ' needs-you' : ''}`)
    .attr('tabindex', 0)
    .attr('aria-label', (d) => `${nodeName(d.session)}: ${sessionStatus(d.session)}. Show it.`)
    .each(function (d) {
      const g = d3.select(this);
      const width = d.w - 14;
      g.select('rect.hit')
        .attr('x', -d.w / 2 + 4)
        .attr('y', -d.h / 2 + 4)
        .attr('width', Math.max(0, d.w - 8))
        .attr('height', Math.max(0, d.h - 8))
        .attr('rx', 10);
      g.select('text.oname').text(fitText(nameBeside(d.session, home), width));
      g.select('text.ostatus').text(fitText(d.session.needs_you ? 'Needs you' : STATE_LABEL[sessionState(d.session)] || '', width));
    });
  // The button that zooms out to them: only with some to show, and orange
  // while one needs you (pulsing when one first does).
  const button = G.host && G.host.querySelector('.show-others');
  if (button) {
    const others = otherSessions();
    button.hidden = !others.length;
    const needing = new Set(others.filter((s) => s.needs_you).map((s) => s.id));
    button.classList.toggle('needs-you', needing.size > 0);
    button.title = !needing.size
      ? 'All sessions'
      : `All sessions: ${needing.size === 1 ? 'another needs you' : `${needing.size} others need you`}`;
    button.setAttribute('aria-label', button.title);
    if ([...needing].some((id) => !othersNeeding.has(id))) pulseButton(button);
    othersNeeding = needing;
  }
}

/** The other sessions that needed you when last drawn: one that newly does pulses the All button. */
let othersNeeding = new Set();

/** Makes the All button pulse, for two seconds. */
function pulseButton(button) {
  button.classList.remove('pulse');
  void button.offsetWidth;
  button.classList.add('pulse');
  clearTimeout(button.pulseTimer);
  button.pulseTimer = setTimeout(() => button.classList.remove('pulse'), 2000);
}

/** The summary of another session, beside the pointer (or the session, from the keyboard). */
function showOtherTip(e, d) {
  const tip = G.host && G.host.querySelector('.graph-tip');
  if (!tip) return;
  const s = d.session;
  const done = s.tasks - s.open_tasks;
  const meta = [appName(s.provider), s.agents ? plural(s.agents, 'agent') : null, s.tasks ? `tasks ${done}/${s.tasks}` : null, ago(s.last_event_at)]
    .filter(Boolean)
    .join(' · ');
  redraw(tip, [
    h('div', { class: 'tip-head' }, h('span', { class: `dot ${sessionState(s)}` }), h('strong', null, nodeName(s))),
    h('div', { class: s.needs_you ? 'tip-attention' : 'tip-muted' }, sessionStatus(s)),
    h('div', { class: 'tip-sub' }, meta),
    h('div', { class: 'tip-hint' }, 'Click to show it'),
  ]);
  placeTip(tip, e);
}

/**
 * Shows session `d` (one of the others): it glides to the middle of the
 * graph and grows as everything else fades, then its own tree is shown.
 */
function goToSession(d) {
  const d3 = window.d3;
  hideGraphTip();
  if (!d3 || !G.svg || G.switching) return;
  G.switching = true;
  G.following = null;
  const chosen = G.svg.selectAll('g.onode').classed('chosen', (o) => o.id === d.id).filter((o) => o.id === d.id);
  // Everything else fades at once, and stays gone, whatever redraws the
  // page meanwhile (said on the elements, not by the host's class).
  const fade = matchMedia('(prefers-reduced-motion: reduce)').matches ? 0 : 250;
  G.svg
    .selectAll('g.onode')
    .filter((o) => o.id !== d.id)
    .classed('gone', true)
    .interrupt()
    .transition('fade')
    .duration(fade)
    .ease(d3.easeCubicOut)
    .style('opacity', 0);
  G.svg.selectAll('g.nodes, g.links').transition('fade').duration(fade).ease(d3.easeCubicOut).style('opacity', 0);
  let went = false;
  const go = () => {
    if (went) return;
    went = true;
    G.switching = false;
    // Where it is on screen, and how large, for its tree to come from (see `arriveFrom`).
    const shape = chosen.select('path.shape').node();
    const at = shape && shape.getBoundingClientRect();
    // And its colours, which its own node's may not be (it's orange while
    // something in it needs you, its node the colour of its own state).
    const look = shape && getComputedStyle(shape);
    G.arrive =
      at && at.width
        ? { id: d.id, x: at.left + at.width / 2, y: at.top + at.height / 2, r: at.width / 2, fill: look && look.fill, stroke: look && look.stroke }
        : null;
    // A copy of it stays on screen while its tree comes (which can take a
    // moment), until its own node's drawn under it.
    showGhost(chosen.node());
    selectRoot(d.id);
  };
  const box = G.svg.node().getBoundingClientRect();
  if (matchMedia('(prefers-reduced-motion: reduce)').matches || !box.width) {
    go();
    return;
  }
  const t = d3.zoomTransform(G.svg.node());
  const [mx, my] = t.invert([box.width / 2, box.height / 2]);
  chosen
    .interrupt()
    .transition()
    .duration(550)
    // Coming to rest there, as its own node sets off from rest.
    .ease(d3.easeCubicInOut)
    .attrTween('transform', function () {
      const k = (this.agPos && this.agPos.k) || d.s;
      return moveTo(this, mx, my, k * 2.2);
    })
    .on('end interrupt', go);
}

/**
 * A copy of `el` (a session chosen from the others), drawn over the page just
 * where it is, so nothing flickers while the graph's redrawn under it. It's
 * faded out by `dropGhost`: when the session's own node's in its place, or
 * after a while, whatever happens.
 */
function showGhost(el) {
  dropGhost(true);
  const svg = G.svg && G.svg.node();
  const viewport = svg && svg.querySelector('g.viewport');
  if (!el || !viewport) return;
  const box = svg.getBoundingClientRect();
  const ns = 'http://www.w3.org/2000/svg';
  const ghost = document.createElementNS(ns, 'svg');
  ghost.setAttribute('class', 'graph-ghost');
  ghost.setAttribute('aria-hidden', 'true');
  Object.assign(ghost.style, { left: `${box.left}px`, top: `${box.top}px`, width: `${box.width}px`, height: `${box.height}px` });
  const g = document.createElementNS(ns, 'g');
  g.setAttribute('transform', viewport.getAttribute('transform') || '');
  g.append(el.cloneNode(true));
  ghost.append(g);
  document.body.append(ghost);
  G.ghost = ghost;
  G.ghostTimer = setTimeout(() => dropGhost(), 3000);
}

/** Fades the copy `showGhost` made away (at once, when `now`). */
function dropGhost(now) {
  const ghost = G.ghost;
  G.ghost = null;
  clearTimeout(G.ghostTimer);
  if (!ghost) return;
  if (now || matchMedia('(prefers-reduced-motion: reduce)').matches) {
    ghost.remove();
    return;
  }
  ghost.classList.add('gone');
  setTimeout(() => ghost.remove(), 300);
}

/** Zooms the tree out into the middle of the graph, and shows the other sessions round it (the "All sessions" button). */
function showOthers() {
  const d3 = window.d3;
  if (!d3 || !G.svg) return;
  G.userMoved = true;
  G.following = null;
  const box = G.svg.node().getBoundingClientRect();
  const nodes = G.sim ? G.sim.nodes() : [];
  if (box.width && box.height && nodes.length) {
    const x0 = Math.min(...nodes.map((n) => n.x - n.room));
    const x1 = Math.max(...nodes.map((n) => n.x + n.room));
    const y0 = Math.min(...nodes.map((n) => n.y - n.r - 10));
    const y1 = Math.max(...nodes.map((n) => n.y + n.r + 22));
    // Small in the middle (a quarter of the way across at most), leaving the rest to the others.
    const k = Math.min(OVERVIEW_IN, (box.width * 0.25) / (x1 - x0), (box.height * 0.25) / (y1 - y0));
    const t = d3.zoomIdentity.translate(box.width / 2 - (k * (x0 + x1)) / 2, box.height / 2 - (k * (y0 + y1)) / 2).scale(k);
    // Placed once the tree's where it's going, so they're round it there.
    G.svg.call(G.zoom.transform, t);
  }
  setOverview(true);
}

// ---------- node dialog ----------

/** Whether event `e` is on a node's tasks pill (which opens the node's details at its tasks). */
function onPill(e) {
  const at = e && e.target;
  return Boolean(at && at.closest && at.closest('g.pill'));
}

/** Wide enough for the details pane at the right of the graph (see app.css). */
const WIDE = matchMedia('(min-width: 1101px)');

/**
 * Shows node `id`'s details (brought to its section headed `at`, if it's
 * given): in the pane at the right, where there's one; otherwise in the
 * dialog (between a phone's width and that, there's no pane: see app.css).
 */
function showDetails(id, at) {
  if (!WIDE.matches) {
    openModal(id, at);
    return;
  }
  hideGraphTip();
  selectNode(id);
  const pane = $('#detail');
  const heading = at && [...pane.querySelectorAll('h3')].find((h3) => h3.textContent.startsWith(at));
  if (heading) heading.scrollIntoView({ block: 'start' });
  else pane.scrollTop = 0;
}

/**
 * The dialog with everything about node `id`: what the details pane shows,
 * centred over the page; brought to its section headed `at` (as "Tasks"),
 * if there is one.
 */
function openModal(id, at) {
  hideGraphTip();
  S.selected = id;
  S.modal = id;
  let dialog = $('#node-modal');
  if (!dialog) {
    dialog = h('dialog', { id: 'node-modal', class: 'node-modal', 'aria-label': 'Details' });
    // A click on the backdrop (the dialog itself, outside its box) closes it.
    dialog.addEventListener('click', (e) => {
      if (e.target === dialog) closeModal();
    });
    dialog.addEventListener('close', () => {
      S.modal = null;
    });
    document.body.append(dialog);
  }
  renderModal();
  if (!dialog.open) {
    if (typeof dialog.showModal === 'function') dialog.showModal();
    else dialog.setAttribute('open', '');
  }
  renderView();
  const heading = at && [...dialog.querySelectorAll('h3')].find((h3) => h3.textContent.startsWith(at));
  if (heading) {
    heading.scrollIntoView({ block: 'start' });
    const part = heading.closest('section') || heading;
    part.classList.remove('spot');
    void part.offsetWidth;
    part.classList.add('spot');
  } else dialog.scrollTop = 0;
}

/** Redraws the open dialog: its node, as of the step being viewed. */
function renderModal() {
  const dialog = $('#node-modal');
  if (!dialog || !S.modal) return;
  const pane = h('div');
  drawDetail(pane, S.modal);
  redraw(dialog, [
    h(
      'div',
      { class: 'modal-box' },
      h('button', { type: 'button', class: 'modal-close', 'aria-label': 'Close', onclick: closeModal }, '×'),
      ...pane.childNodes,
    ),
  ]);
}

function closeModal() {
  const dialog = $('#node-modal');
  if (dialog && dialog.open) {
    if (typeof dialog.close === 'function') dialog.close();
    else dialog.removeAttribute('open');
  }
  S.modal = null;
}

/** A download link for a PNG of this session, at the step being viewed. */
function saveImageLink(stop) {
  const dark = matchMedia('(prefers-color-scheme: dark)').matches;
  const url = source.imageUrl && source.imageUrl(S.root, !S.following && stop ? stop.id : null, dark);
  if (!url) return null;
  const when = !S.following && stop ? new Date(stop.ts) : new Date();
  const time = when.toISOString().slice(0, 19).replace(/[-:]/g, '').replace('T', '-');
  const name = `agent-graph-${short(S.root)}-${time}.png`;
  return h(
    'a',
    {
      class: 'btn',
      href: url,
      download: name,
      title: 'Save a picture of this session as it looks here',
      onclick: source.fetchImage
        ? (e) => {
            e.preventDefault();
            saveImage(url, name);
          }
        : null,
    },
    'Save image',
  );
}

/** Downloads the image at `url` as `name`, through `source.fetchImage`. */
async function saveImage(url, name) {
  try {
    const href = URL.createObjectURL(await source.fetchImage(url));
    const link = h('a', { href, download: name, hidden: true });
    document.body.append(link);
    link.click();
    link.remove();
    setTimeout(() => URL.revokeObjectURL(href), 60_000);
  } catch (e) {
    setError(`Couldn't save the image: ${e.message}`);
  }
}

/** A card and its children's cards. `drawn` skips any node already drawn. */
function branch(graph, node, ringed, flash, keep, drawn = new Set()) {
  drawn.add(node.id);
  const kids = node.children
    .map((id) => graph.nodes[id])
    .filter((k) => k && !drawn.has(k.id) && !hidden(graph, k, keep));
  kids.forEach((k) => drawn.add(k.id));
  return h(
    'div',
    { class: 'branch' },
    card(graph, node, ringed, flash),
    kids.length ? h('div', { class: 'children' }, kids.map((k) => branch(graph, k, ringed, flash, keep, drawn))) : null,
  );
}

function card(graph, n, ringed, flash) {
  const classes = ['node', n.state];
  if (n.id === S.selected) classes.push('selected');
  if (n.id === ringed) classes.push('current');
  if (n.id === flash) classes.push('flash');
  const doneTasks = n.tasks.length - n.open_tasks;

  return h(
    'button',
    // Its details: in the pane at the right, or on a phone a page of their
    // own; between, where there's neither, in the dialog.
    { class: classes.join(' '), 'data-id': n.id, onclick: () => (WIDE.matches || NARROW.matches ? selectNode(n.id) : openModal(n.id)) },
    h('span', { class: `dot ${n.state}` }),
    h(
      'span',
      { class: 'node-main' },
      h(
        'span',
        { class: 'node-head' },
        h('span', { class: 'node-name' }, n.kind === 'session' && !n.parent && !n.title ? 'Session' : nodeName(n)),
        h('span', { class: `state ${n.state}` }, STATE_LABEL[n.state]),
        n.background ? h('span', { class: 'chip' }, 'background') : null,
        n.stale ? h('span', { class: 'chip warn', title: 'No events for a while; it may have crashed' }, 'stale?') : null,
      ),
      n.purpose && (n.kind === 'agent' || n.parent) ? h('span', { class: 'node-purpose' }, n.purpose) : null,
      n.state === 'input_required'
        ? h('span', { class: 'node-attention' }, `Needs you: ${n.attention || 'waiting for input'}`)
        : n.headline
          ? h('span', { class: 'node-headline' }, n.headline)
          : null,
      n.blocked ? h('span', { class: 'node-blocked' }, blockedText(graph, n.blocked)) : null,
    ),
    n.tasks.length
      ? h(
          'span',
          { class: 'progress', title: `${doneTasks} of ${n.tasks.length} tasks done` },
          `${doneTasks}/${n.tasks.length}`,
          h('span', { class: 'bar' }, barFill(doneTasks / n.tasks.length)),
        )
      : null,
  );
}

function barFill(fraction) {
  const fill = h('span');
  fill.style.width = `${Math.round(fraction * 100)}%`;
  return fill;
}

function blockedText(graph, b) {
  const parts = [];
  if (b.cycle) parts.push('Deadlock');
  if (b.on.length) {
    const names = b.on
      .slice(0, 2)
      .map((id) => nameOf(graph, id) + (nodeOf(graph, id) && nodeOf(graph, id).stale ? ' (looks stuck)' : ''));
    if (b.on.length > 2) names.push(`${b.on.length - 2} more`);
    parts.push(`Waiting on ${names.join(', ')}`);
  }
  if (b.starting) parts.push(`${plural(b.starting, 'agent')} starting`);
  if (b.open_tasks) parts.push(`${plural(b.open_tasks, 'open task')} ahead`);
  return parts.join(' · ');
}

function renderDetail() {
  const pane = h('div');
  drawDetail(pane);
  redraw($('#detail'), [...pane.childNodes]);
}

/** Draws what's known of the selected node, as of the step shown, into `pane`. */
/** What's known about node `id` (the one selected, unless another's given). */
function drawDetail(pane, id = S.selected) {
  const graph = S.shown || S.live;
  const n = graph && id ? graph.nodes[id] : null;
  if (!n) {
    pane.append(
      h(
        'p',
        { class: 'placeholder' },
        id && S.live && S.live.nodes[id]
          ? 'This hadn’t started yet at this point in the timeline.'
          : 'Select a session or agent to see its tasks, waits and messages.',
      ),
    );
    return;
  }

  // A node of this tree is shown here; one outside it, by showing its own tree.
  const link = (id) =>
    graph.nodes[id]
      ? h('button', { class: 'linkish', 'data-id': id, onclick: () => selectNode(id) }, nameOf(graph, id))
      : nodeOf(graph, id)
        ? h('button', { class: 'linkish', 'data-id': id, title: 'Show its own tree', onclick: () => switchTo(id) }, nameOf(graph, id))
        : short(id);

  pane.append(
    h(
      'div',
      { class: 'detail-head' },
      h('h2', null, n.kind === 'session' && !n.parent ? `Session · ${nodeName(n)}` : nodeName(n), h('span', { class: `state ${n.state}` }, STATE_LABEL[n.state])),
      h(
        'div',
        { class: 'idline' },
        h('span', { class: 'mono', title: n.id }, n.id),
        h('button', { class: 'copy', onclick: (e) => copy(n.id, e.currentTarget) }, 'Copy'),
      ),
    ),
  );

  const openBox = renderOpen(n.id);
  if (openBox) pane.append(openBox);

  if (n.state === 'input_required') {
    pane.append(h('div', { class: 'attention-box' }, `Needs you: ${n.attention || 'waiting for input'}`));
  }
  if (n.purpose || n.headline) {
    pane.append(h('p', { class: 'lead' }, n.purpose || n.headline));
    if (n.purpose && n.headline) pane.append(h('p', { class: 'node-headline' }, n.headline));
  }

  if (n.blocked) {
    pane.append(
      section(
        'Waiting on',
        h(
          'ul',
          { class: 'list' },
          n.blocked.on.map((id) =>
            h('li', null, h('span', { class: `dot ${(nodeOf(graph, id) || {}).state || 'idle'}` }), h('span', { class: 'grow' }, link(id))),
          ),
          n.blocked.starting ? h('li', null, h('span', { class: 'grow sub' }, `${plural(n.blocked.starting, 'agent')} starting…`)) : null,
        ),
        h('p', { class: 'node-headline' }, `${plural(n.blocked.nodes, 'unfinished node')} and ${plural(n.blocked.open_tasks, 'open task')} before this can continue.`),
      ),
    );
  }

  if (n.tasks.length) {
    const icon = { completed: '✓', in_progress: '▸', pending: '○' };
    pane.append(
      section(
        `Tasks · ${n.tasks.length - n.open_tasks}/${n.tasks.length} done`,
        h(
          'ul',
          { class: 'list' },
          n.tasks.map((t) =>
            h(
              'li',
              { class: `task ${t.status}` },
              h('span', { class: 'task-icon', 'aria-label': t.status.replace('_', ' ') }, icon[t.status] || '○'),
              h('span', { class: 'grow' }, t.status === 'in_progress' && t.active_text ? t.active_text : t.text),
            ),
          ),
        ),
      ),
    );
  }

  if (n.spawns.length) {
    pane.append(
      section(
        n.spawns.some((s) => s.kind === 'session') ? 'Agents and sessions it started' : 'Agents it started',
        h(
          'ul',
          { class: 'list' },
          n.spawns.map((s) =>
            h(
              'li',
              null,
              h('span', { class: `dot ${s.child && nodeOf(graph, s.child) ? nodeOf(graph, s.child).state : s.returned ? 'completed' : 'working'}` }),
              h(
                'span',
                { class: 'grow' },
                s.child ? link(s.child) : `${s.agent_type || 'Agent'} (starting…)`,
                h('span', { class: 'sub' }, [s.purpose, s.background ? 'background' : 'waited for'].filter(Boolean).join(' · ')),
              ),
              h('span', { class: 'time' }, clock(s.requested_at)),
            ),
          ),
        ),
      ),
    );
  }

  if (n.messages.length) {
    pane.append(
      section(
        'Messages',
        h(
          'ul',
          { class: 'list' },
          n.messages.map((m) =>
            h(
              'li',
              null,
              h('span', { class: 'task-icon', title: m.direction }, m.direction === 'sent' ? '↗' : '↙'),
              h(
                'span',
                { class: 'grow' },
                m.direction === 'sent' ? 'To ' : 'From ',
                link(m.peer),
                m.summary ? h('span', { class: 'sub' }, m.summary) : null,
                m.body ? h('div', { class: 'body-text' }, m.body) : null,
              ),
              h('span', { class: 'time' }, clock(m.ts)),
            ),
          ),
        ),
      ),
    );
  }

  const facts = [
    [
      'Kind',
      n.provider === 'run'
        ? 'A command, run by agent-graph run'
        : n.kind === 'session'
          ? 'Session'
          : `${n.agent_type || 'Agent'}${n.background ? ' (background)' : ''}`,
    ],
    ['Provider', n.provider],
    ['Parent', n.parent ? link(n.parent) : null],
    ['Linked by', LINK_LABEL[n.link]],
    ['Folder', n.cwd],
    ['Started', n.started_at && clock(n.started_at)],
    ['Ended', n.ended_at && clock(n.ended_at)],
    ['Last event', clock(n.last_event_at)],
  ].filter(([, v]) => v);
  pane.append(section('Details', h('dl', { class: 'facts' }, facts.map(([k, v]) => [h('dt', null, k), h('dd', null, v)]))));
}

/**
 * A button that reopens the session in its agent, if this page can (see
 * `source.open`). Whether it can comes from the live graph, even when looking
 * back: it's about the session now.
 */
function renderOpen(id) {
  const offer = source.open && S.live && S.live.open && S.live.open[id];
  if (!offer) return null;
  // Kept in S, so the outcome survives the panel redrawing as events arrive.
  const done = S.opened && S.opened.id === id ? S.opened : null;
  const button = h(
    'button',
    {
      class: 'btn',
      disabled: Boolean(done && done.busy),
      title: offer.desktop
        ? `Show it in ${offer.app}, as it is.`
        : offer.copy
          ? 'It hasn’t ended, so it’s probably open somewhere already. This opens a copy of its conversation in a new terminal window.'
          : 'Resume it in a new terminal window.',
      onclick: () => openSession(id),
    },
    offer.desktop ? `Open in ${offer.app}` : offer.copy ? `Open a copy in ${offer.app}` : `Resume in ${offer.app}`,
  );
  let note = null;
  if (done && done.error) {
    note = h(
      'div',
      { class: 'open-note' },
      h('span', { class: 'open-error' }, done.error),
      done.command
        ? [
            h('span', { class: 'open-command' }, 'Run it yourself:'),
            h(
              'div',
              { class: 'idline' },
              h('code', { class: 'mono', title: done.command }, done.command),
              h('button', { class: 'copy', onclick: (e) => copy(done.command, e.currentTarget) }, 'Copy'),
            ),
          ]
        : null,
    );
  } else if (done && !done.busy) {
    const where = offer.desktop ? `Opened in ${offer.app}.` : 'Opened in a new terminal window.';
    note = h('div', { class: 'open-note' }, h('span', { class: 'open-ok' }, where));
  }
  return h('div', { class: 'open-box' }, button, note);
}

async function openSession(id) {
  S.opened = { id, busy: true };
  renderDetail();
  try {
    await source.open(id);
    S.opened = { id };
  } catch (err) {
    S.opened = { id, error: err.message || 'Couldn’t open it.', command: err.command };
  }
  renderDetail();
}

/** How a session found the session that started it (`node.link`). */
const LINK_LABEL = {
  env: 'Inherited from its parent’s shell',
  run: 'agent-graph run',
  process: 'Its processes (the parent’s agent started it)',
  suggested: 'Its name (the parent suggested it, and you started it)',
};

function section(title, ...body) {
  return h('section', null, h('h3', null, title), ...body);
}

async function copy(text, button) {
  try {
    await navigator.clipboard.writeText(text);
    button.textContent = 'Copied';
    setTimeout(() => (button.textContent = 'Copy'), 1200);
  } catch {
    button.textContent = 'Copy failed';
  }
}

// ---------- timeline ----------

let tickKey = '';

function renderTimeline() {
  const slider = $('#slider');
  const n = S.stops.length;
  slider.disabled = n < 2;
  slider.max = String(Math.max(0, n - 1));
  if (document.activeElement !== slider || slider.value !== String(S.pos)) slider.value = String(Math.max(0, S.pos));
  const pct = n > 1 ? (S.pos / (n - 1)) * 100 : 100;
  slider.style.setProperty('--pct', `${pct}%`);
  const stop = S.stops[S.pos];
  slider.setAttribute('aria-valuetext', stop ? `Step ${S.pos + 1} of ${n}: ${clean(stop.label)}` : 'No events');

  // Ticks only change when the stops do (or the track's width). Too many
  // for the track to show apart (under about 5px each) would only be a
  // solid block: then there are none.
  const width = $('#track').clientWidth;
  const crowded = n > 1 && width > 0 && width / n < 5;
  const key = `${S.root}|${n}|${n ? S.stops[n - 1].id : ''}|${crowded}`;
  const ticks = $('#ticks');
  if (key !== tickKey) {
    tickKey = key;
    ticks.replaceChildren(
      ...(crowded ? [] : S.stops).map((s, i) => {
        const t = h('span', { class: `tick ${s.category}` });
        t.style.left = `${n > 1 ? (i / (n - 1)) * 100 : 100}%`;
        return t;
      }),
    );
  }
  ticks.childNodes.forEach((t, i) => {
    t.classList.toggle('ahead', i > S.pos);
  });

  $('#prev').disabled = S.pos <= 0;
  $('#next').disabled = S.pos >= n - 1;
  const live = $('#live');
  live.classList.toggle('on', S.following && S.connected);
  $('#live-text').textContent = S.following ? source.liveLabel : `Go to ${source.liveLabel.toLowerCase()}`;

  $('#step-count').textContent = n ? `Step ${S.pos + 1} of ${n}` : 'No events yet';
  $('#step-time').textContent = stop ? clock(stop.ts) : '';
  $('#step-label').replaceChildren(...(stop ? [h('span', { class: `swatch ${stop.category}` }), h('span', null, stop.label)] : []));
}

function stopAt(clientX) {
  const rect = $('#slider').getBoundingClientRect();
  const n = S.stops.length;
  if (n < 2) return 0;
  const x = clamp((clientX - rect.left - THUMB / 2) / (rect.width - THUMB), 0, 1);
  return Math.round(x * (n - 1));
}

function showTip(e) {
  const tip = $('#hover-tip');
  if (S.stops.length < 2) return;
  const i = stopAt(e.clientX);
  const stop = S.stops[i];
  const rect = $('#track').getBoundingClientRect();
  const when = h('span', { class: 't' }, `${i + 1} · ${clock(stop.ts)}`);
  tip.replaceChildren(when);
  tip.hidden = false;
  // At most 80% of the window's width: a long label keeps its start and its
  // end, with an ellipsis in the middle.
  const style = getComputedStyle(tip);
  const room = window.innerWidth * 0.8 - (parseFloat(style.paddingLeft) || 0) - (parseFloat(style.paddingRight) || 0) - when.offsetWidth - 6;
  tip.append(middleEllipsis(clean(stop.label), room, style.font));
  // Over its step, but on the screen.
  const x = THUMB / 2 + (i / (S.stops.length - 1)) * (rect.width - THUMB);
  const half = tip.offsetWidth / 2;
  const lo = half + 8 - rect.left;
  const hi = window.innerWidth - rect.left - half - 8;
  tip.style.left = `${clamp(x, lo, Math.max(lo, hi))}px`;
}

/** Text measured in `font`, from a canvas (about 7px a character where there's none). */
function textWidth(text, font) {
  const ctx = (textWidth.ctx ||= document.createElement('canvas').getContext('2d'));
  if (!ctx) return text.length * 7;
  ctx.font = font;
  return ctx.measureText(text).width;
}

/** `text` cut to `px` wide in `font`, if it's wider: its start and end, with an ellipsis between. */
function middleEllipsis(text, px, font) {
  if (textWidth(text, font) <= px) return text;
  // The most characters that fit, half from each end.
  let lo = 0;
  let hi = text.length;
  while (lo < hi) {
    const n = Math.ceil((lo + hi) / 2);
    const cut = `${text.slice(0, Math.ceil(n / 2))}…${text.slice(text.length - Math.floor(n / 2))}`;
    if (textWidth(cut, font) <= px) lo = n;
    else hi = n - 1;
  }
  return `${text.slice(0, Math.ceil(lo / 2)).trimEnd()}…${text.slice(text.length - Math.floor(lo / 2)).trimStart()}`;
}

// ---------- wiring ----------

function connect() {
  source.subscribe(scheduleRefresh, (connected) => {
    const reconnected = connected && !S.connected;
    S.connected = connected;
    renderMode();
    renderTimeline();
    if (reconnected) scheduleRefresh(); // catch up on anything missed while disconnected
  });
}

function wire() {
  const slider = $('#slider');
  slider.addEventListener('input', () => goTo(Number(slider.value)));
  slider.addEventListener('mousemove', showTip);
  slider.addEventListener('mouseleave', () => ($('#hover-tip').hidden = true));
  $('#prev').addEventListener('click', () => goTo(S.pos - 1));
  $('#next').addEventListener('click', () => goTo(S.pos + 1));
  $('#live').addEventListener('click', goLive);
  // The timeline drawer, on a phone.
  $('#timeline-hide').addEventListener('click', () => setTimelineOpen(false));
  window.addEventListener('resize', () => {
    renderTimelineDrawer();
    renderTimeline();
  });
  // "Completed", in the list's head and, on a session's page (where the
  // list isn't shown), in the top bar: one setting, two boxes kept alike.
  const showDone = [$('#show-done'), $('#show-done-page')];
  for (const box of showDone) {
    box.checked = S.showDone;
    box.addEventListener('change', (e) => {
      S.showDone = e.target.checked;
      for (const other of showDone) other.checked = S.showDone;
      try {
        localStorage.setItem('agentGraphShowDone', String(S.showDone));
      } catch {
        // Not remembered, then; it still applies now.
      }
      renderSessions();
      renderMain();
    });
  }
  // The Apps dropdown closes on a click outside it, or Escape.
  const appFilter = $('#app-filter');
  appFilter.addEventListener('toggle', () => {
    if (appFilter.open) placeAppMenu(appFilter);
  });
  // It's placed on the page, not in the list (which would clip it): kept
  // under its button as the list scrolls, or the window changes size.
  const replace = () => {
    if (appFilter.open) placeAppMenu(appFilter);
  };
  window.addEventListener('resize', replace);
  document.addEventListener('scroll', replace, true);
  document.addEventListener('click', (e) => {
    if (appFilter.open && !appFilter.contains(e.target)) appFilter.open = false;
  });
  appFilter.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && appFilter.open) {
      appFilter.open = false;
      appFilter.querySelector('summary').focus();
    }
  });
  $('#show-all').addEventListener('change', (e) => {
    S.showAll = e.target.checked;
    renderSessions();
  });

  document.addEventListener('keydown', (e) => {
    if (e.metaKey || e.ctrlKey || e.altKey || S.modal) return;
    const t = e.target;
    if (t === slider || (t instanceof HTMLElement && t.matches('input, textarea, select'))) return;
    if (e.key === 'ArrowLeft') goTo(S.pos - 1);
    else if (e.key === 'ArrowRight') goTo(S.pos + 1);
    else if (e.key === 'Home') goTo(0);
    else if (e.key === 'End' || e.key === 'l' || e.key === 'L') goLive();
    else return;
    e.preventDefault();
  });

  // A session's page: the back button, and the browser's Back, go to the
  // list, where the session is brought into view.
  $('#back').addEventListener('click', closePage);
  window.addEventListener('popstate', () => {
    const was = document.body.classList.contains('paged');
    const update = () => {
      // The list, as left: a reload stays on it.
      const ours = history.state && (history.state.agentGraphPage || history.state.agentGraphList);
      if (!ours) history.replaceState({ ...history.state, agentGraphList: true }, '', location.href);
      renderPage();
      if (paged()) window.scrollTo(0, 0);
      else {
        const item = document.querySelector('.session.selected');
        if (item) item.scrollIntoView({ block: 'center' });
      }
    };
    // The name moves back to the list (or, going forward, to the heading).
    if (was && !paged()) transition(update, headingName(), listedName);
    else if (!was && paged()) transition(update, listedName(), headingName);
    else update();
  });
  NARROW.addEventListener('change', renderPage);

  // Another node named in the address: its tree, whether or not this graph
  // has it (if it doesn't exist, the refresh shows the newest session).
  window.addEventListener('hashchange', () => {
    const id = hashId();
    if (!id || id === S.root) return;
    // Not one this graph has: if it doesn't exist, come back here.
    S.returnTo = known(id) ? null : S.root;
    switchTo(id);
    // On a phone, as its page (a link to it was followed, say).
    openPage();
  });

  // Keep "2m ago" fresh.
  setInterval(renderSessions, 30_000);
}

async function main() {
  wire();
  // On a phone, a session named in the address opens as its page, with the
  // list a step back; not where the list was left (reloaded, say), whose
  // address names the session last shown.
  // (The site's framework keeps state of its own in the entry: only ours
  // counts. It can also write the entry over as it starts, dropping ours:
  // then it's put back, unless the reader has moved on meanwhile.)
  const ours = history.state && (history.state.agentGraphPage || history.state.agentGraphList);
  if (NARROW.matches && hashId() && !ours) {
    history.pushState({ ...history.state, agentGraphPage: true }, '', location.href);
    let moved = false;
    window.addEventListener('popstate', () => (moved = true), { once: true });
    for (const ms of [200, 800, 2000, 5000]) {
      setTimeout(() => {
        if (moved || paged() || !NARROW.matches) return;
        history.replaceState({ ...history.state, agentGraphPage: true }, '', location.href);
        renderPage();
        renderView();
      }, ms);
    }
  }
  renderAll();
  try {
    S.info = await source.info();
    if (S.info.now_ms) S.skew = S.info.now_ms - Date.now();
  } catch {
    /* shown by the first refresh instead */
  }
  connect();
  scheduleRefresh();
}

main();
}
