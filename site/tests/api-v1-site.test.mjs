// The graph API (/api/v1, site/openapi.json), end to end, against a
// running copy of the site and the Auth and Firestore emulators: keys made
// and revoked on the account page, a live share shared as watch-remote
// shares one, and read back through every endpoint.
//
//   npm run emulators            # in one terminal
//   FIREBASE_AUTH_EMULATOR_HOST=127.0.0.1:9099 npm run dev   # in another
//   BASE_URL=http://localhost:3000 npm run test:api
//
// tests/api-v1.test.mts tests the endpoints themselves more closely,
// without the site.

import assert from "node:assert/strict";
import {createHash, randomBytes} from "node:crypto";
import {readdirSync, readFileSync} from "node:fs";
import {test} from "node:test";

const BASE = (process.env.BASE_URL || "http://localhost:3000").replace(
  /\/$/,
  "",
);
const AUTH = process.env.FIREBASE_AUTH_EMULATOR_HOST || "127.0.0.1:9099";
const FROM_SITE = {Origin: BASE};

const secret = () => randomBytes(32).toString("base64url");

// ---------- accounts, and a live share (as tests/accounts.test.mjs makes them) ----------

async function loggedIn() {
  const email = `t-${randomBytes(6).toString("hex")}@agent-graph.test`;
  const res = await fetch(
    `http://${AUTH}/identitytoolkit.googleapis.com/v1/accounts:signUp?key=demo-key`,
    {
      method: "POST",
      headers: {"Content-Type": "application/json"},
      body: JSON.stringify({
        email,
        password: secret(),
        returnSecureToken: true,
      }),
    },
  );
  assert.equal(res.status, 200, await res.clone().text());
  const {idToken, localId} = await res.json();
  const session = await fetch(`${BASE}/api/session`, {
    method: "POST",
    headers: {"Content-Type": "application/json", ...FROM_SITE},
    body: JSON.stringify({idToken}),
  });
  assert.equal(session.status, 204, await session.clone().text());
  const cookie = session.headers
    .getSetCookie()
    .map(c => c.split(";")[0])
    .join("; ");
  return {email, uid: localId, cookie};
}

/** An account logged in in a browser, and on a computer (the CLI's token). */
async function withCli() {
  const browser = await loggedIn();
  const verifier = secret();
  const challenge = createHash("sha256").update(verifier).digest("base64url");
  const codeRes = await fetch(`${BASE}/api/cli/code`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Cookie: browser.cookie,
      ...FROM_SITE,
    },
    body: JSON.stringify({port: 43210, state: secret(), challenge}),
  });
  assert.equal(codeRes.status, 200, await codeRes.clone().text());
  const code = new URL((await codeRes.json()).redirect).searchParams.get(
    "code",
  );
  const tokenRes = await fetch(`${BASE}/api/cli/token`, {
    method: "POST",
    headers: {"Content-Type": "application/json"},
    body: JSON.stringify({code, verifier, host: "test-computer"}),
  });
  assert.equal(tokenRes.status, 200, await tokenRes.clone().text());
  return {...browser, token: (await tokenRes.json()).token};
}

const EXAMPLES = new URL("../../examples/logs/events/", import.meta.url);
const LOG = readdirSync(EXAMPLES)
  .filter(f => f.endsWith(".jsonl"))
  .sort()
  .map(f => readFileSync(new URL(f, EXAMPLES), "utf8"))
  .join("");
const LINES = LOG.split(/(?<=\n)/);
const FIRST_HALF = LINES.slice(0, 60).join("");
const SECOND_HALF = LINES.slice(60).join("");

/** A live share of the examples' first half, said to be running (so it's listed). */
async function liveShare(owner, host = "laptop") {
  const res = await fetch(`${BASE}/api/logs`, {
    method: "POST",
    headers: {
      "Content-Type": "application/x-ndjson",
      "X-Agent-Graph-Source": "watch",
      Authorization: `Bearer ${owner.token}`,
    },
    body: FIRST_HALF,
  });
  assert.equal(res.status, 201, await res.clone().text());
  const log = await res.json();
  const alive = await fetch(`${BASE}/api/logs/${log.id}/alive`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${log.writeToken}`,
    },
    body: JSON.stringify({sessions: 1, host, summary: {}}),
  });
  assert.equal(alive.status, 204, await alive.clone().text());
  return {...log, offset: Buffer.byteLength(FIRST_HALF)};
}

async function append(log, text) {
  const res = await fetch(
    `${BASE}/api/logs/${log.id}/append?offset=${log.offset}`,
    {
      method: "POST",
      headers: {
        "Content-Type": "application/x-ndjson",
        Authorization: `Bearer ${log.writeToken}`,
      },
      body: text,
    },
  );
  assert.equal(res.status, 204, await res.clone().text());
  log.offset += Buffer.byteLength(text);
}

// ---------- keys, on the account page ----------

function makeKey(cookie, body, headers = FROM_SITE) {
  return fetch(`${BASE}/api/account/api-keys`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      ...headers,
      ...(cookie ? {Cookie: cookie} : {}),
    },
    body: JSON.stringify(body),
  });
}

async function newKey(cookie, body = {name: "optimizer"}) {
  const res = await makeKey(cookie, body);
  assert.equal(res.status, 201, await res.clone().text());
  return res.json();
}

const api = (path, key, headers = {}) =>
  fetch(`${BASE}/api/v1${path}`, {
    headers: {...(key ? {Authorization: `Bearer ${key}`} : {}), ...headers},
  });

async function apiOk(path, key, headers) {
  const res = await api(path, key, headers);
  const body = await res.json();
  assert.equal(res.status, 200, `${path}: ${JSON.stringify(body)}`);
  return body;
}

async function apiFails(path, key, status, code) {
  const res = await api(path, key);
  const body = await res.json();
  assert.equal(res.status, status, `${path}: ${JSON.stringify(body)}`);
  assert.equal(body.error.code, code, path);
  assert.match(res.headers.get("request-id"), /^req_/);
  return body.error;
}

test("keys are made, listed and revoked on the account page, only by its account", async () => {
  const owner = await loggedIn();
  assert.equal((await makeKey(null, {name: "x"})).status, 401, "logged in");
  assert.equal(
    (await makeKey(owner.cookie, {name: "x"}, {Origin: "https://evil.test"}))
      .status,
    403,
    "from the site's own pages",
  );
  assert.equal((await makeKey(owner.cookie, {name: " "})).status, 400);
  assert.equal(
    (await makeKey(owner.cookie, {name: "x".repeat(101)})).status,
    400,
  );
  assert.equal(
    (await makeKey(owner.cookie, {name: "x", kind: "admin"})).status,
    400,
  );
  assert.equal(
    (await makeKey(owner.cookie, {name: "x", kind: "restricted"})).status,
    400,
    "a restricted key needs graphs",
  );
  assert.equal(
    (await makeKey(owner.cookie, {name: "x", graphs: ["gph_AbCdEf123456"]}))
      .status,
    400,
    "only a restricted key is made for graphs",
  );

  const made = await newKey(owner.cookie, {name: "optimizer", kind: "secret"});
  assert.match(made.key, /^ag_sk_live_[A-Za-z0-9]{32}$/);
  assert.equal(made.shown, `${made.key.slice(0, 11)}****${made.key.slice(-4)}`);
  assert.equal(made.kind, "secret");
  assert.equal(made.graphs, null);
  assert.equal(made.version, "2026-10-09");

  const listed = await (
    await fetch(`${BASE}/api/account/api-keys`, {
      headers: {Cookie: owner.cookie},
    })
  ).json();
  assert.deepEqual(
    listed.keys.map(k => [k.id, k.name, k.kind, k.shown]),
    [[made.id, "optimizer", "secret", made.shown]],
  );
  assert.ok(
    !JSON.stringify(listed).includes(made.key),
    "never the key itself again",
  );
  assert.equal((await fetch(`${BASE}/api/account/api-keys`)).status, 401);

  // The account page shows it.
  const page = await (
    await fetch(`${BASE}/account`, {headers: {Cookie: owner.cookie}})
  ).text();
  assert.match(page, /API keys/);
  assert.ok(page.includes(made.shown));

  // It works, until it's revoked, then not at all, at once.
  await apiOk("/graphs", made.key);
  const other = await loggedIn();
  const revoke = (cookie, id = made.id) =>
    fetch(`${BASE}/api/account/api-keys/${id}`, {
      method: "DELETE",
      headers: {...FROM_SITE, ...(cookie ? {Cookie: cookie} : {})},
    });
  assert.equal((await revoke(other.cookie)).status, 404, "not theirs");
  assert.equal((await revoke(null)).status, 401);
  assert.equal((await revoke(owner.cookie, "nope")).status, 404);
  assert.equal((await revoke(owner.cookie)).status, 204);
  await apiFails("/graphs", made.key, 401, "api_key_invalid");
  assert.equal((await revoke(owner.cookie)).status, 404, "once");
});

test("a CLI login isn't a key to the API, nor is nothing", async () => {
  const owner = await withCli();
  await apiFails("/graphs", owner.token, 401, "api_key_invalid");
  await apiFails("/graphs", null, 401, "api_key_missing");
  const res = await api("/graphs", null, {
    Authorization: `Bearer ${"ag_sk_live_" + "x".repeat(32)}`,
  });
  assert.equal(res.status, 401);
  assert.equal(
    res.headers.get("content-type"),
    "application/json; charset=utf-8",
  );
});

test("a live share, read through every endpoint, as it goes on", async () => {
  const owner = await withCli();
  const log = await liveShare(owner);
  const {key} = await newKey(owner.cookie);
  const G = `gph_${log.id}`;

  // Listed, and its own.
  const list = await apiOk("/graphs", key);
  assert.deepEqual(
    list.data.map(g => g.id),
    [G],
  );
  const graph = await apiOk(`/graphs/${G}`, key);
  assert.equal(graph.object, "graph");
  assert.equal(graph.source, "watch");
  assert.equal(graph.url, `${BASE}/l/${log.id}`);
  const half = graph.retention.event_count;
  assert.ok(half > 0 && half <= 60);
  // By its link's id too, and by Basic auth.
  const basic = Buffer.from(`${key}:`).toString("base64");
  assert.equal(
    (await apiOk(`/graphs/${log.id}`, null, {Authorization: `Basic ${basic}`}))
      .id,
    G,
  );

  // The rest of it arrives: the next request sees it.
  await append(log, SECOND_HALF);
  const later = await apiOk(`/graphs/${G}`, key);
  assert.ok(later.retention.event_count > half);
  assert.ok(later.counts.nodes >= graph.counts.nodes);

  // Nodes: listed, searched, and one, expanded, through the site's routes.
  const nodes = await apiOk(`/graphs/${G}/nodes?order=tree&limit=1000`, key);
  assert.equal(nodes.data.length, later.counts.nodes);
  const child = nodes.data.find(n => n.parent_id);
  assert.ok(child, "a node with a parent");
  const one = await apiOk(
    `/graphs/${G}/nodes/${child.id}?expand[]=parent&expand[]=root&expand[]=tasks`,
    key,
  );
  assert.equal(one.id, child.id);
  assert.equal(one.parent.id, child.parent_id);
  assert.equal(one.root.id, child.root_id);
  assert.ok(Array.isArray(one.tasks));
  const root = await apiOk(
    `/graphs/${G}/nodes/${child.root_id}?expand[]=descendants`,
    key,
  );
  assert.equal(root.descendants.data.length, root.descendant_count);
  const viaUrl = await apiOk(
    root.descendants.url.replace(/^\/api\/v1/, ""),
    key,
  );
  assert.deepEqual(
    viaUrl.data.map(n => n.id),
    root.descendants.data.slice(0, 100).map(n => n.id),
  );
  const found = await apiOk(
    `/graphs/${G}/nodes/search?query=${encodeURIComponent('kind:"session"')}`,
    key,
  );
  assert.ok(
    found.total_count > 0 && found.data.every(n => n.kind === "session"),
  );

  // Events, oldest first, each with what it changed; and one, fixed for good.
  const events = await apiOk(`/graphs/${G}/events?order=asc&limit=1000`, key);
  assert.equal(events.data.length, later.retention.event_count);
  assert.equal(events.data[0].changes[0].created, true);
  const first = events.data[0].id;
  const event = await api(`/graphs/${G}/events/${first}?expand[]=node`, key);
  assert.equal(event.status, 200);
  assert.equal(
    event.headers.get("cache-control"),
    "private, max-age=31536000, immutable",
  );
  assert.equal((await event.json()).node.id, events.data[0].node_id);
  const atFirst = await apiOk(`/graphs/${G}?as_of=${first}`, key);
  assert.equal(atFirst.counts.nodes, 1);

  // Unchanged: a 304.
  const res = await api(`/graphs/${G}/nodes`, key);
  const etag = res.headers.get("etag");
  assert.ok(etag);
  assert.equal(res.headers.get("agent-graph-version"), "2026-10-09");
  assert.ok(res.headers.get("ratelimit-limit"));
  const again = await api(`/graphs/${G}/nodes`, key, {"If-None-Match": etag});
  assert.equal(again.status, 304);

  // Errors, in the API's shape, through the site.
  await apiFails(
    `/graphs/${G}/nodes/${"node_" + "x".repeat(10)}`,
    key,
    404,
    "resource_missing",
  );
  await apiFails(`/graphs/${G}/nodes?limit=0`, key, 400, "parameter_invalid");
  await apiFails(
    `/graphs/${G}/events/evt_ZZZZZZZZZZZZZZZZZZZZZZZZZZ`,
    key,
    404,
    "resource_missing",
  );
  await apiFails("/nope", key, 404, "resource_missing");
  await apiFails(`/graphs/${G}/nodes/x/y`, key, 404, "resource_missing");
  const versioned = await api("/graphs", key, {
    "Agent-Graph-Version": "1999-01-01",
  });
  assert.equal(versioned.status, 400);
  assert.equal((await versioned.json()).error.code, "version_invalid");
});

test("another account's graph isn't there; a restricted key reads only its own", async () => {
  const owner = await withCli();
  const laptop = await liveShare(owner, "laptop");
  const cloud = await liveShare(owner, "cloud");
  const stranger = await loggedIn();
  const theirs = await newKey(stranger.cookie);
  await apiFails(
    `/graphs/gph_${laptop.id}`,
    theirs.key,
    404,
    "resource_missing",
  );
  assert.deepEqual((await apiOk("/graphs", theirs.key)).data, []);

  // A stranger can't make a key for it either.
  assert.equal(
    (
      await makeKey(stranger.cookie, {
        name: "x",
        kind: "restricted",
        graphs: [`gph_${laptop.id}`],
      })
    ).status,
    400,
  );
  const restricted = await newKey(owner.cookie, {
    name: "laptop only",
    kind: "restricted",
    graphs: [`gph_${laptop.id}`],
  });
  assert.match(restricted.key, /^ag_rk_live_/);
  assert.deepEqual(restricted.graphs, [`gph_${laptop.id}`]);
  await apiOk(`/graphs/gph_${laptop.id}`, restricted.key);
  await apiFails(
    `/graphs/gph_${cloud.id}`,
    restricted.key,
    403,
    "api_key_restricted",
  );
  assert.deepEqual(
    (await apiOk("/graphs", restricted.key)).data.map(g => g.id),
    [`gph_${laptop.id}`],
  );
  // A secret key reads both.
  const all = await newKey(owner.cookie);
  assert.deepEqual(
    (await apiOk("/graphs", all.key)).data.map(g => g.id).sort(),
    [`gph_${laptop.id}`, `gph_${cloud.id}`].sort(),
  );
});

test("deleting an account takes its keys", async () => {
  const owner = await loggedIn();
  const {key} = await newKey(owner.cookie);
  await apiOk("/graphs", key);
  const res = await fetch(`${BASE}/api/account`, {
    method: "DELETE",
    headers: {
      "Content-Type": "application/json",
      ...FROM_SITE,
      Cookie: owner.cookie,
    },
    body: JSON.stringify({confirm: owner.email}),
  });
  assert.equal(res.status, 204, await res.clone().text());
  await apiFails("/graphs", key, 401, "api_key_invalid");
});

/** POST /api/showcase/key, as the showcase asks it. */
const showcaseKeyOf = (source, headers = {}) =>
  fetch(`${BASE}/api/showcase/key`, {
    method: "POST",
    headers: {"Content-Type": "application/json", ...FROM_SITE, ...headers},
    body: JSON.stringify({source}),
  });

test("the API showcase is served at /showcase, and hands out keys only as it should", async () => {
  for (const path of ["/showcase", "/showcase/g/explorer?graph=gph_x"]) {
    const res = await fetch(`${BASE}${path}`);
    assert.equal(res.status, 200, path);
    assert.match(res.headers.get("content-type") ?? "", /text\/html/, path);
    assert.match(await res.text(), /Agent Graph API · showcase/, path);
  }
  // Without a login, there's no account of one's own to read.
  const mine = await showcaseKeyOf("mine");
  assert.equal(mine.status, 401);
  assert.equal((await mine.json()).error.code, "not_logged_in");
  // Nor for another site.
  const elsewhere = await showcaseKeyOf("demo", {Origin: "https://evil.test"});
  assert.equal(elsewhere.status, 403);
  // Under CI, SHOWCASE_API_KEY isn't set (scripts/ci-api-test.sh).
  if (process.env.SHOWCASE_API_KEY) return;
  const demo = await showcaseKeyOf("demo");
  assert.equal(demo.status, 503);
  assert.equal((await demo.json()).error.code, "showcase_not_configured");
});

test("a logged-in browser is given a key to its own graphs, which isn't listed", async () => {
  const owner = await withCli();
  const log = await liveShare(owner);
  const res = await showcaseKeyOf("mine", {Cookie: owner.cookie});
  assert.equal(res.status, 200, await res.clone().text());
  assert.equal(res.headers.get("cache-control"), "no-store");
  const {key, expires} = await res.json();
  assert.match(key, /^ag_sk_live_/);
  const hour = 60 * 60 * 1000;
  assert.ok(Math.abs(expires - (Date.now() + hour)) < 5 * 60 * 1000);

  const graphs = await fetch(`${BASE}/api/v1/graphs`, {
    headers: {Authorization: `Bearer ${key}`},
  });
  assert.equal(graphs.status, 200, await graphs.clone().text());
  assert.deepEqual(
    (await graphs.json()).data.map(g => g.id),
    [`gph_${log.id}`],
  );

  const listed = await fetch(`${BASE}/api/account/api-keys`, {
    headers: {Cookie: owner.cookie},
  });
  assert.deepEqual((await listed.json()).keys, [], "not on the account page");
});
