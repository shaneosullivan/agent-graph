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
  connected: false,
  info: null,
  cache: new Map(), // event id -> graph at that stop, least recently shown first (a new map on each refresh)
  seq: 0, // bumped whenever the view moves on, so a step still loading isn't shown
  lastFlashed: null,
  scrolledTo: {}, // { graph, id }: the ringed card last brought into view
  error: null,
  skew: 0, // server clock minus ours; non-zero when AGENT_GRAPH_NOW pins it
  opened: null, // { id, busy?, error?, command? }: the last Open button press
  returnTo: null, // where to go back to if a node named in the address doesn't exist
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

function nodeName(node) {
  if (!node) return 'Unknown';
  if (node.kind === 'session') {
    if (node.title) return node.title;
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
    history.replaceState(null, '', `#${encodeURIComponent(back)}`);
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
  if (known(id)) switchTo(id);
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
  history.replaceState(null, '', `#${encodeURIComponent(id)}`);
  scheduleRefresh();
  renderAll();
}

function selectNode(id) {
  S.selected = id;
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
  return S.live.roots.filter(
    (id) => S.showAll || id === S.root || new Date(S.live.sessions[id].last_event_at).getTime() >= cutoff,
  );
}

function setError(message) {
  S.error = message;
  const banner = $('#banner');
  banner.hidden = !message;
  banner.textContent = message || '';
}

// ---------- rendering ----------

function renderAll() {
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
  redraw($('#session-list'), sessionItems());
}

function sessionItems() {
  if (!S.live) return [];
  const roots = visibleRoots();
  if (!roots.length) {
    const hidden = S.live.roots.length;
    return [h('li', { class: 'empty-note' }, hidden ? `${plural(hidden, 'older session')} hidden.` : 'No sessions yet.')];
  }
  return roots.map((id) => {
    // What's going on in the session's tree, worked out where the graph is.
    const root = S.live.sessions[id];
    const { agents, needs_you: needsYou, deadlocked, stuck, busy } = root;
    const dotState = needsYou ? 'input_required' : busy && root.state === 'idle' ? 'working' : root.state;
    const done = root.tasks - root.open_tasks;
    const meta = [root.provider, agents ? plural(agents, 'agent') : null, root.tasks ? `tasks ${done}/${root.tasks}` : null]
      .filter(Boolean)
      .join(' · ');
    return h(
      'li',
      null,
      h(
        'button',
        {
          class: `session${id === S.root ? ' selected' : ''}${needsYou ? ' needs-you' : ''}`,
          'data-id': id,
          'aria-current': id === S.root ? 'true' : null,
          onclick: () => selectRoot(id),
        },
        h('span', { class: `dot ${dotState}` }),
        h('span', { class: 's-title', title: root.cwd || id }, nodeName(root)),
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
  });
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
  // Brought into view when the step changes; not whenever the tree is drawn
  // again (a card chosen, new events), or it would undo the reader's scrolling.
  const ring = view.querySelector('.node.current');
  if (ring && (S.scrolledTo.graph !== graph || S.scrolledTo.id !== ringed)) ring.scrollIntoView({ block: 'nearest' });
  S.scrolledTo = { graph, id: ringed };
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
          h('h2', null, 'Nothing in the last 24 hours'),
          h('p', null, 'Tick “Older” in the sessions list to see earlier sessions.'),
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
      h('h1', null, nodeName(liveRoot), root ? h('span', { class: `state ${root.state}` }, STATE_LABEL[root.state]) : null),
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

  return { kids: [head, h('div', { class: 'tree' }, branch(graph, root, ringed, flash))], graph, ringed, flash };
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
function branch(graph, node, ringed, flash, drawn = new Set()) {
  drawn.add(node.id);
  const kids = node.children.map((id) => graph.nodes[id]).filter((k) => k && !drawn.has(k.id));
  kids.forEach((k) => drawn.add(k.id));
  return h(
    'div',
    { class: 'branch' },
    card(graph, node, ringed, flash),
    kids.length ? h('div', { class: 'children' }, kids.map((k) => branch(graph, k, ringed, flash, drawn))) : null,
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
      title: offer.copy
        ? 'It hasn’t ended, so it’s probably open somewhere already. This opens a copy of its conversation in a new terminal window.'
        : 'Resume it in a new terminal window.',
      onclick: () => openSession(id),
    },
    offer.copy ? `Open a copy in ${offer.app}` : `Resume in ${offer.app}`,
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
    note = h('div', { class: 'open-note' }, h('span', { class: 'open-ok' }, 'Opened in a new terminal window.'));
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

  // Another node named in the address: its tree, whether or not this graph
  // has it (if it doesn't exist, the refresh shows the newest session).
  window.addEventListener('hashchange', () => {
    const id = hashId();
    if (!id || id === S.root) return;
    // Not one this graph has: if it doesn't exist, come back here.
    S.returnTo = known(id) ? null : S.root;
    switchTo(id);
  });

  // Keep "2m ago" fresh.
  setInterval(renderSessions, 30_000);
}

async function main() {
  wire();
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
