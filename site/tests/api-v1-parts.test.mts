// Unit tests for the graph API's parts (lib/api/): its ids, reading
// queries, paging, searching, expanding, rate limits, keys in requests and
// errors' shape:  npm run test:unit

import assert from "node:assert/strict";
import {register} from "node:module";
import {test} from "node:test";

register("../scripts/resolve-ts.mjs", import.meta.url);
const ids = await import("../lib/api/ids.ts");
const {parseQuery, inRange, LIST_PARAMS} = await import("../lib/api/query.ts");
const {paginate} = await import("../lib/api/paginate.ts");
const {parseSearch, matches, SEARCH_FIELDS} =
  await import("../lib/api/search.ts");
const {parseExpand} = await import("../lib/api/expand.ts");
const {RateLimiter} = await import("../lib/api/ratelimit.ts");
const {credentialOf} = await import("../lib/api/handler.ts");
const {ApiError, apiKeyInvalid, outsideRetention, rateLimited} =
  await import("../lib/api/errors.ts");
const {shownKey, KEY_PATTERN} = await import("../lib/api/keys.ts");
type ApiNode = import("../lib/api/model.ts").ApiNode;

/** What `fn` threw, as an ApiError (failing if it threw nothing, or something else). */
function thrown(fn: () => unknown): InstanceType<typeof ApiError> {
  try {
    fn();
  } catch (err) {
    assert.ok(err instanceof ApiError, `not an ApiError: ${err}`);
    return err;
  }
  assert.fail("nothing was thrown");
}

test("ids: each object's id, and back, with no lookup", () => {
  assert.equal(ids.graphId("AbCdEf123456"), "gph_AbCdEf123456");
  assert.equal(ids.parseGraphId("gph_AbCdEf123456"), "AbCdEf123456");
  // A share link's id works too.
  assert.equal(ids.parseGraphId("AbCdEf123456"), "AbCdEf123456");
  for (const bad of [
    "gph_short",
    "gph_AbCdEf12345!",
    "",
    "gph_",
    "AbCdEf1234567",
  ]) {
    assert.equal(ids.parseGraphId(bad), null, bad);
  }

  const ref =
    "claude-code:5f2c1e9a-8d1b-4b6e-9a51-0f6f2c7d3e10/a3fd07c2b19e4d61";
  const node = ids.nodeId(ref);
  assert.match(node, /^node_[A-Za-z0-9_-]+$/);
  assert.equal(ids.parseNodeId(node), ref);
  // Unicode too, and nothing that isn't written as nodeId writes it.
  assert.equal(ids.parseNodeId(ids.nodeId("x:ünï/😀")), "x:ünï/😀");
  for (const bad of ["node_", "node_!!", "nod_abc", `${node}=`, "node_Y2xh="]) {
    assert.equal(ids.parseNodeId(bad), null, bad);
  }

  assert.equal(
    ids.eventId("01K0000000000000000000000A"),
    "evt_01K0000000000000000000000A",
  );
  assert.equal(
    ids.parseEventId("evt_01K0000000000000000000000A"),
    "01K0000000000000000000000A",
  );
  for (const bad of ["evt_", "evt_a~1", "01K0", "evt_a b"]) {
    assert.equal(ids.parseEventId(bad), null, bad);
  }
});

const SPEC = {
  ...LIST_PARAMS,
  as_of: {kind: "asOf"},
  expand: {kind: "strings", max: 3},
  state: {kind: "enums", values: ["working", "idle"]},
  stale: {kind: "bool"},
  order: {kind: "enum", values: ["newest", "tree"]},
  created: {kind: "range"},
  name: {kind: "string", max: 5},
} as const;

const q = (query: string) =>
  parseQuery(new URL(`https://x.test/p?${query}`), SPEC);

test("a query: arrays and ranges in brackets, each value of its kind", () => {
  assert.deepEqual(q(""), {});
  assert.deepEqual(
    q("expand[]=parent&expand[]=root&state[]=idle&stale=true&limit=5"),
    {expand: ["parent", "root"], state: ["idle"], stale: true, limit: 5},
  );
  // Stripe's other ways of writing arrays: plain, and indexed.
  assert.deepEqual(q("expand=a&expand[1]=b").expand, ["a", "b"]);
  assert.deepEqual(q("created[gte]=10&created[lt]=20").created, {
    gte: 10,
    lt: 20,
  });
  assert.deepEqual(q("as_of=evt_01K0000000000000000000000A").as_of, {
    event: "01K0000000000000000000000A",
  });
  assert.deepEqual(q("as_of=1760004843221").as_of, {time: 1760004843221});
  assert.equal(q("order=tree").order, "tree");
  assert.equal(q("starting_after=node_abc").starting_after, "node_abc");
});

test("a query: what isn't taken, or can't be read, is refused, naming it", () => {
  const cases: Array<[string, string, string]> = [
    ["nope=1", "parameter_unknown", "nope"],
    ["limit[]=1", "parameter_unknown", "limit[]"],
    ["expand[x]=a", "parameter_unknown", "expand[x]"],
    ["limit=0", "parameter_invalid", "limit"],
    ["limit=1001", "parameter_invalid", "limit"],
    ["limit=ten", "parameter_invalid", "limit"],
    ["limit=1&limit=2", "parameter_invalid", "limit"],
    ["stale=yes", "parameter_invalid", "stale"],
    ["order=oldest", "parameter_invalid", "order"],
    ["state[]=asleep", "parameter_invalid", "state"],
    ["created=5", "parameter_invalid", "created"],
    ["created[gte]=soon", "parameter_invalid", "created[gte]"],
    ["created[around]=5", "parameter_invalid", "created"],
    ["as_of=yesterday", "parameter_invalid", "as_of"],
    [
      "expand[]=a&expand[]=b&expand[]=c&expand[]=d",
      "parameter_invalid",
      "expand",
    ],
    ["name=", "parameter_invalid", "name"],
    ["name=toolong", "parameter_invalid", "name"],
  ];
  for (const [query, code, param] of cases) {
    const e = thrown(() => q(query));
    assert.equal(e.status, 400, query);
    assert.equal(e.code, code, query);
    assert.equal(e.param, param, query);
  }
});

test("ranges", () => {
  assert.ok(inRange(5, undefined));
  assert.ok(inRange(5, {gte: 5, lte: 5}));
  assert.ok(!inRange(5, {gt: 5}));
  assert.ok(!inRange(5, {lt: 5}));
  assert.ok(!inRange(null, {gt: 0}));
  assert.ok(inRange(null, undefined));
});

test("paging: after an object, before one, and whether there's more", () => {
  const items = ["a", "b", "c", "d", "e"];
  const id = (s: string) => s;
  assert.deepEqual(paginate(items, id, {limit: 2}), {
    data: ["a", "b"],
    has_more: true,
  });
  assert.deepEqual(paginate(items, id, {limit: 2, starting_after: "b"}), {
    data: ["c", "d"],
    has_more: true,
  });
  assert.deepEqual(paginate(items, id, {limit: 2, starting_after: "d"}), {
    data: ["e"],
    has_more: false,
  });
  assert.deepEqual(paginate(items, id, {limit: 2, ending_before: "e"}), {
    data: ["c", "d"],
    has_more: true,
  });
  assert.deepEqual(paginate(items, id, {limit: 2, ending_before: "b"}), {
    data: ["a"],
    has_more: false,
  });
  assert.deepEqual(paginate(items, id, {}), {data: items, has_more: false});
  assert.equal(
    thrown(() => paginate(items, id, {starting_after: "z"})).param,
    "starting_after",
  );
  assert.equal(
    thrown(() => paginate(items, id, {starting_after: "a", ending_before: "c"}))
      .param,
    "ending_before",
  );
  // Walking forwards visits each once.
  const seen: Array<string> = [];
  let after: string | undefined;
  for (;;) {
    const page = paginate(items, id, {
      limit: 2,
      ...(after ? {starting_after: after} : {}),
    });
    seen.push(...page.data);
    if (!page.has_more) break;
    after = page.data.at(-1);
  }
  assert.deepEqual(seen, items);
});

const node = (over: Partial<ApiNode>): ApiNode =>
  ({
    id: "node_a",
    object: "node",
    as_of_event_id: null,
    graph_id: "gph_x",
    provider_ref: "x:a",
    provider: "claude-code",
    kind: "agent",
    parent_id: "node_p",
    root_id: "node_r",
    session_id: "node_r",
    requested_by_id: null,
    depth: 1,
    state: "working",
    stale: false,
    attention: null,
    title: null,
    summary: "Running the Migration tests",
    headline: null,
    purpose: null,
    agent_type: "Explore",
    background: false,
    link_method: null,
    spawn_call_id: null,
    cwd: null,
    created: 1000,
    ended: null,
    last_event: 2000,
    child_count: 0,
    descendant_count: 12,
    background_agent_count: 0,
    task_counts: {total: 3, pending: 1, in_progress: 1, completed: 1, open: 2},
    blocked: {
      on_ids: [],
      starting: 0,
      node_count: 1,
      open_tasks: 25,
      cycle: false,
    },
    ...over,
  }) as ApiNode;

test("search: the query language, clause by clause", () => {
  const n = node({});
  const yes = [
    'state:"working"',
    'state:"WORKING"',
    'stale:"false"',
    'blocked:"true"',
    'agent_type:"Explore" AND descendant_count>10',
    'state:"failed" OR state:"working"',
    'summary~"migration"',
    'summary~"MIGRATION" AND -state:"completed"',
    "blocked_open_tasks>=20",
    "open_tasks:2",
    "depth<2",
    "depth<=1",
    "created>999",
    "ended:null",
    "title:null",
    'parent_id:"node_p"',
    'root_id:"node_r" AND session_id:"node_r"',
    'background:"false"',
    "-ended:null OR depth:1",
  ];
  for (const query of yes) {
    assert.ok(matches(parseSearch(query), n), query);
  }
  const no = [
    'state:"idle"',
    'summary~"nothing here"',
    "descendant_count>12",
    "-depth:1",
    'parent_id:"node_x"',
    'stale:"true"',
    'attention~"abc"',
    "ended>0",
    'kind:"session"',
  ];
  for (const query of no) {
    assert.ok(!matches(parseSearch(query), n), query);
  }
  assert.ok(
    !matches(parseSearch("blocked_open_tasks>0"), node({blocked: null})),
  );
  assert.ok(matches(parseSearch('blocked:"false"'), node({blocked: null})));
  // Quoted text, with escaped quotes.
  assert.ok(
    matches(parseSearch('summary:"say \\"hi\\""'), node({summary: 'say "hi"'})),
  );
  assert.ok(SEARCH_FIELDS.includes("blocked_open_tasks"));
});

test("search: a query that can't be read says why, and where", () => {
  const bad: Array<[string, RegExp]> = [
    ["", /empty/],
    ['state:"working" AND', /clause after AND/],
    ['state:"a" AND kind:"b" OR depth:1', /not both/],
    ["nope:1", /can't be searched/],
    ["state", /Expected :/],
    ["state:", /Expected a value/],
    ["state:working", /Quote working/],
    ['state:"working', /isn't closed/],
    ['summary~"ab"', /at least 3/],
    ['depth~"abc"', /isn't text/],
    ['state>"a"', /needs a number/],
    ["state>3", /needs a number/],
    ['stale:"maybe"', /"true" or "false"/],
    ['depth:"deep"', /is a number/],
    ["depth>null", /depth:null/],
    ['state:"a" state:"b"', /Expected AND or OR at character 11/],
    ['state:"a" & depth:1', /Unexpected &/],
  ];
  for (const [query, message] of bad) {
    const e = thrown(() => parseSearch(query));
    assert.equal(e.code, "search_query_invalid", query);
    assert.equal(e.param, "query");
    assert.match(e.message, message, query);
  }
});

test("expand: paths, where each can be used, and how deep", () => {
  const trie = parseExpand(
    ["parent", "children.children", "blocked.on", "root.tasks", "descendants"],
    "node",
  );
  assert.deepEqual(
    [...trie.keys()],
    ["parent", "children", "blocked", "root", "descendants"],
  );
  assert.ok(trie.get("children")?.has("children"));
  assert.ok(trie.get("blocked")?.has("on"));
  assert.equal(parseExpand(undefined, "node").size, 0);
  assert.deepEqual(
    [...parseExpand(["data.parent", "data.blocked.on"], "nodes").keys()],
    ["parent", "blocked"],
  );
  assert.ok(parseExpand(["node.parent"], "event").get("node")?.has("parent"));
  assert.ok(parseExpand(["data.node.root"], "events").get("node")?.has("root"));
  // Four levels deep is allowed, after data. in a list too.
  parseExpand(["children.children.children.children"], "node");
  parseExpand(["data.parent.parent.parent.parent"], "nodes");

  const bad: Array<[Array<string>, string, string]> = [
    [
      ["children.children.children.children.children"],
      "node",
      "expand_too_deep",
    ],
    [["data.parent.parent.parent.parent.parent"], "nodes", "expand_too_deep"],
    [["parent"], "nodes", "expand_invalid"],
    [["data"], "nodes", "expand_invalid"],
    [["data.children"], "nodes", "expand_not_allowed"],
    [["data.descendants"], "nodes", "expand_not_allowed"],
    [["data.node.children"], "events", "expand_not_allowed"],
    [["children.descendants"], "node", "expand_not_allowed"],
    [["node.descendants"], "event", "expand_not_allowed"],
    [["descendants.parent"], "node", "expand_invalid"],
    [["tasks.parent"], "node", "expand_invalid"],
    [["blocked"], "node", "expand_invalid"],
    [["blocked.nodes"], "node", "expand_invalid"],
    [["graph"], "node", "expand_invalid"],
    [["parent_id"], "node", "expand_invalid"],
    [["payload"], "event", "expand_invalid"],
    [["Parent"], "node", "expand_invalid"],
    [Array.from({length: 21}, () => "parent"), "node", "parameter_invalid"],
  ];
  for (const [paths, at, code] of bad) {
    const e = thrown(() => parseExpand(paths, at as "node"));
    assert.equal(e.code, code, paths.join());
    assert.equal(e.param, "expand");
  }
});

test("rate limits: a burst, then the rate, and a 304 given back", () => {
  let now = 0;
  const limiter = new RateLimiter(25, 100, () => now);
  for (let i = 0; i < 100; i++) {
    assert.ok(limiter.take("k").ok, `request ${i}`);
  }
  const refused = limiter.take("k");
  assert.equal(refused.ok, false);
  assert.equal(refused.remaining, 0);
  assert.equal(refused.retryAfter, 1);
  assert.equal(refused.limit, 25);
  // Another key has its own.
  assert.ok(limiter.take("other").ok);
  now += 1000;
  for (let i = 0; i < 25; i++) {
    assert.ok(limiter.take("k").ok, `a second later, ${i}`);
  }
  assert.equal(limiter.take("k").ok, false);
  limiter.giveBack("k");
  assert.ok(limiter.take("k").ok, "given back");
  assert.equal(limiter.take("k").reset, 4, "seconds until it's full again");
  now += 60_000;
  assert.equal(limiter.take("k").remaining, 99, "full again, less this one");
});

test("a request's key: a bearer token, or Basic auth's username", () => {
  const req = (authorization?: string) =>
    new Request("https://x.test/", {
      headers: authorization ? {authorization} : {},
    });
  const key = "ag_sk_live_0123456789abcdefghijABCDEFGHIJKL";
  assert.equal(credentialOf(req(`Bearer ${key}`)), key);
  assert.equal(credentialOf(req(`bearer   ${key}`)), key);
  const basic = Buffer.from(`${key}:`).toString("base64");
  assert.equal(credentialOf(req(`Basic ${basic}`)), key);
  assert.equal(
    credentialOf(req(`Basic ${Buffer.from(`${key}:pw`).toString("base64")}`)),
    key,
  );
  assert.equal(credentialOf(req()), null);
  assert.equal(credentialOf(req("Bearer")), null);
  assert.equal(credentialOf(req("Token abc")), null);
  assert.equal(
    credentialOf(req(`Basic ${Buffer.from(":pw").toString("base64")}`)),
    null,
  );
});

test("keys: their shape, and how they're shown", () => {
  const key = "ag_rk_live_0123456789abcdefghijABCDEFGHIJKL";
  assert.match(key, KEY_PATTERN);
  assert.equal(shownKey(key), "ag_rk_live_****IJKL");
  for (const bad of [
    "ag_sk_test_0123456789abcdefghijABCDEFGHIJKL",
    "agt_x",
    "ag_sk_live_short",
  ]) {
    assert.doesNotMatch(bad, KEY_PATTERN);
  }
});

test("errors: one shape, with where each is explained", () => {
  const body = apiKeyInvalid("ag_sk_live_****IJKL").body(
    "https://x.test/docs/reference",
  );
  assert.deepEqual(body, {
    error: {
      type: "authentication_error",
      code: "api_key_invalid",
      message: "Invalid API key: ag_sk_live_****IJKL.",
      param: null,
      doc_url: "https://x.test/docs/reference#errors",
    },
  });
  const gone = outsideRetention("evt_A", "evt_B");
  assert.equal(gone.status, 410);
  assert.equal(gone.body("d").error.first_event_id, "evt_A");
  assert.equal(gone.body("d").error.last_event_id, "evt_B");
  assert.equal(rateLimited(2).headers["Retry-After"], "2");
});
