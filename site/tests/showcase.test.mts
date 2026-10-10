// lib/showcase.ts: the API showcase's proxy (/showcase/api/ag/…), with the
// API it calls in-process faked.

import assert from "node:assert/strict";
import {test} from "node:test";

import {type Dispatch, showcaseProxy} from "../lib/showcase.ts";

const KEY = "ag_rk_live_demo";

/** A fake API: records what it's asked, and answers `reply`. */
function api(reply: () => Response) {
  const asked: Array<{url: string; headers: Headers; path: Array<string>}> = [];
  const dispatch: Dispatch = async (req, path) => {
    asked.push({url: req.url, headers: req.headers, path});
    return reply();
  };
  return {asked, dispatch};
}

const ok = (headers: Record<string, string> = {}) =>
  new Response('{"object":"graph"}', {
    status: 200,
    headers: {
      "Content-Type": "application/json; charset=utf-8",
      ETag: 'W/"abc"',
      "Request-Id": "req_1",
      "RateLimit-Remaining": "99",
      "Cache-Control": "private, no-cache",
      "Set-Cookie": "nope=1",
      ...headers,
    },
  });

test("without a key, it says the showcase isn't set up, and calls nothing", async () => {
  const {asked, dispatch} = api(ok);
  const res = await showcaseProxy(
    new Request("https://site.test/showcase/api/ag/graphs"),
    ["graphs"],
    {key: null, dispatch},
  );
  assert.equal(res.status, 503);
  assert.equal((await res.json()).error.code, "showcase_not_configured");
  assert.equal(res.headers.get("cache-control"), "no-store");
  assert.equal(asked.length, 0);
});

test("only /graphs is read", async () => {
  const {asked, dispatch} = api(ok);
  const res = await showcaseProxy(
    new Request("https://site.test/showcase/api/ag/account"),
    ["account"],
    {key: KEY, dispatch},
  );
  assert.equal(res.status, 404);
  assert.equal(asked.length, 0);
});

test("it asks the API with the demo's key, and passes back what it should", async () => {
  const {asked, dispatch} = api(ok);
  const res = await showcaseProxy(
    new Request(
      "https://site.test/showcase/api/ag/graphs/gph_1/nodes?expand[]=parent&limit=5",
      {headers: {"If-None-Match": 'W/"old"', Authorization: "Bearer theirs"}},
    ),
    ["graphs", "gph_1", "nodes"],
    {key: KEY, dispatch},
  );
  assert.equal(asked.length, 1);
  const [call] = asked;
  assert.equal(
    call.url,
    "https://site.test/api/v1/graphs/gph_1/nodes?expand[]=parent&limit=5",
  );
  assert.deepEqual(call.path, ["graphs", "gph_1", "nodes"]);
  assert.equal(call.headers.get("authorization"), `Bearer ${KEY}`);
  assert.equal(call.headers.get("if-none-match"), 'W/"old"');

  assert.equal(res.status, 200);
  assert.equal(await res.text(), '{"object":"graph"}');
  assert.equal(res.headers.get("etag"), 'W/"abc"');
  assert.equal(res.headers.get("request-id"), "req_1");
  assert.equal(res.headers.get("ratelimit-remaining"), "99");
  assert.match(res.headers.get("x-upstream-ms") ?? "", /^\d+$/);
  assert.equal(
    res.headers.get("set-cookie"),
    null,
    "only the headers it means to",
  );
  assert.match(
    res.headers.get("cache-control") ?? "",
    /^public, .*s-maxage=60/,
  );
});

test("a fixed point in history is cached for good; an error not at all", async () => {
  const fixed = api(() =>
    ok({"Cache-Control": "private, max-age=31536000, immutable"}),
  );
  const res = await showcaseProxy(
    new Request("https://site.test/showcase/api/ag/graphs/gph_1?as_of=evt_1"),
    ["graphs", "gph_1"],
    {key: KEY, dispatch: fixed.dispatch},
  );
  assert.equal(
    res.headers.get("cache-control"),
    "public, max-age=31536000, immutable",
  );

  const failing = api(
    () =>
      new Response('{"error":{}}', {
        status: 404,
        headers: {"Cache-Control": "no-store"},
      }),
  );
  const missing = await showcaseProxy(
    new Request("https://site.test/showcase/api/ag/graphs/gph_x"),
    ["graphs", "gph_x"],
    {key: KEY, dispatch: failing.dispatch},
  );
  assert.equal(missing.status, 404);
  assert.equal(missing.headers.get("cache-control"), "no-store");
});

test("a 304 is passed back without a body", async () => {
  const {dispatch} = api(
    () => new Response(null, {status: 304, headers: {ETag: 'W/"abc"'}}),
  );
  const res = await showcaseProxy(
    new Request("https://site.test/showcase/api/ag/graphs", {
      headers: {"If-None-Match": 'W/"abc"'},
    }),
    ["graphs"],
    {key: KEY, dispatch},
  );
  assert.equal(res.status, 304);
  assert.equal(await res.text(), "");
  assert.equal(res.headers.get("etag"), 'W/"abc"');
});
