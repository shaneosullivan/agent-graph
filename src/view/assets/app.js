'use strict';

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
 * wherever nothing changed.
 */
function morph(el, kids) {
  const old = [...el.childNodes];
  kids.forEach((kid, i) => {
    const was = old[i];
    if (!was) el.append(kid);
    else if (was.nodeName !== kid.nodeName) was.replaceWith(kid);
    else if (was.nodeType === Node.TEXT_NODE) {
      if (was.data !== kid.data) was.data = kid.data;
    } else {
      for (const { name } of [...was.attributes]) if (!kid.hasAttribute(name)) was.removeAttribute(name);
      for (const { name, value } of [...kid.attributes]) if (was.getAttribute(name) !== value) was.setAttribute(name, value);
      was.onclick = kid.onclick;
      morph(was, [...kid.childNodes]);
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
 * glance. Simple drawings, not the apps' own logos.
 */
const APP_ICON = {
  // A spark, in Claude's orange.
  'claude-code': svgImage(
    '<rect width="16" height="16" rx="4" fill="#D97757"/><path d="M8 3.2v9.6M3.2 8h9.6M4.6 4.6l6.8 6.8M11.4 4.6l-6.8 6.8" stroke="#fff" stroke-width="1.6" stroke-linecap="round"/>',
  ),
  // A prompt.
  codex: svgImage(
    '<rect width="16" height="16" rx="4" fill="#111"/><path d="M4 5.5 6.5 8 4 10.5" fill="none" stroke="#fff" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/><path d="M8 11h4" stroke="#fff" stroke-width="1.6" stroke-linecap="round"/>',
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
  return `${node.agent_type || 'Agent'} ${short(node.id)}`;
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

function renderPage() {
  const on = paged();
  document.body.classList.toggle('paged', on);
  $('#back').hidden = !on;
  $('#page-toggles').hidden = !on;
}

/** Shows the tree under `id`: live, until its timeline comes. */
function switchTo(id) {
  if (id === S.root) return;
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
  renderView();
  // Stacked, its details are below the tree: brought into view.
  if (NARROW.matches) $('#detail').scrollIntoView({ behavior: 'smooth', block: 'start' });
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
  return S.hiddenApps.size > 0 && appsInUse().length > 1;
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
          appsChanged();
        });
        return h('label', null, tick, appIcon(app), h('span', null, appName(app)), h('span', { class: 'count' }, ''));
      }),
    );
  }
  const shown = apps.filter(([app]) => !S.hiddenApps.has(app));
  for (const tick of menu.querySelectorAll('input[data-app]')) {
    const app = tick.dataset.app;
    if (app) {
      tick.checked = !S.hiddenApps.has(app);
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
      : shown.length === 0
        ? 'None'
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
  menu.classList.remove('from-end');
  const button = box.querySelector('summary').getBoundingClientRect();
  const room = document.documentElement.clientWidth - button.left - 8;
  menu.classList.toggle('from-end', menu.getBoundingClientRect().width > room);
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
 * you, centering it, and wrapping around at either end.
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
  const meta = [root.provider, agents ? plural(agents, 'agent') : null, root.tasks ? `tasks ${done}/${root.tasks}` : null]
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
  const { kids, graph, ringed, flash } = mainView();
  redraw(view, kids);
  if (!graph) return;
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

  const head = h(
    'div',
    { class: 'view-head' },
    h(
      'div',
      { class: 'title-row' },
      // Named as it was at the step being viewed: sessions get renamed.
      h('h1', null, h('span', { class: 'h-name' }, nodeName(root || liveRoot)), root ? h('span', { class: `state ${root.state}` }, STATE_LABEL[root.state]) : null),
      root ? saveImageLink(stop) : null,
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
  const tree = h('div', { class: 'tree' }, branch(graph, root, ringed, flash, keep));
  const left = Object.values(graph.nodes).filter((n) => n.id !== S.root && isDone(n) && hidden(graph, n, keep)).length;
  const what = left === 1 ? 'completed agent or session' : 'completed agents and sessions';
  const note = left ? h('p', { class: 'empty-note' }, `${left} ${what} hidden.`) : null;
  return { kids: note ? [head, tree, note] : [head, tree], graph, ringed, flash };
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
    { class: classes.join(' '), 'data-id': n.id, onclick: () => selectNode(n.id) },
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
function drawDetail(pane) {
  const graph = S.shown || S.live;
  const n = graph && S.selected ? graph.nodes[S.selected] : null;
  if (!n) {
    pane.append(
      h(
        'p',
        { class: 'placeholder' },
        S.selected && S.live && S.live.nodes[S.selected]
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
        ? 'Show it in the Claude desktop app’s Code tab.'
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
    const where = offer.desktop ? 'Opened in the Claude app.' : 'Opened in a new terminal window.';
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

  // Ticks only change when the stops do.
  const key = `${S.root}|${n}|${n ? S.stops[n - 1].id : ''}`;
  const ticks = $('#ticks');
  if (key !== tickKey) {
    tickKey = key;
    ticks.replaceChildren(
      ...S.stops.map((s, i) => {
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
  tip.replaceChildren(h('span', { class: 't' }, `${i + 1} · ${clock(stop.ts)}`), clean(stop.label));
  tip.hidden = false;
  const x = THUMB / 2 + (i / (S.stops.length - 1)) * (rect.width - THUMB);
  tip.style.left = `${clamp(x, 120, rect.width - 120)}px`;
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
    if (e.metaKey || e.ctrlKey || e.altKey) return;
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
      if (!history.state) history.replaceState({ agentGraphList: true }, '', location.href);
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
  if (NARROW.matches && hashId() && !history.state) history.pushState({ agentGraphPage: true }, '', location.href);
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
