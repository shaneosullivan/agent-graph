// lib/showcase.ts: the keys the API showcase reads graphs with, from the
// browser (POST /api/showcase/key), with making one faked.

import assert from "node:assert/strict";
import {register} from "node:module";
import {test} from "node:test";

register("../scripts/resolve-ts.mjs", import.meta.url);
const {showcaseKeyReply} = await import("../lib/showcase.ts");

const DEMO = "ag_rk_live_demo";

/** A fake key maker: records whom it made keys for. */
function minter() {
  const made: Array<string> = [];
  const mint = async (uid: string) => {
    made.push(uid);
    return {key: `ag_sk_live_for_${uid}`, expires: 1234};
  };
  return {made, mint};
}

test("the demo's key is the site's, for anyone", async () => {
  const {made, mint} = minter();
  const res = await showcaseKeyReply("demo", {demoKey: DEMO, uid: null, mint});
  assert.equal(res.status, 200);
  assert.deepEqual(await res.json(), {key: DEMO, expires: null});
  assert.equal(res.headers.get("cache-control"), "no-store");
  assert.equal(made.length, 0);
});

test("without a demo key, the showcase says it isn't set up", async () => {
  const {mint} = minter();
  const res = await showcaseKeyReply("demo", {demoKey: null, uid: "u", mint});
  assert.equal(res.status, 503);
  assert.equal((await res.json()).error.code, "showcase_not_configured");
});

test("a browser's own key is made for its account, only once it's logged in", async () => {
  const {made, mint} = minter();
  const out = await showcaseKeyReply("mine", {demoKey: DEMO, uid: null, mint});
  assert.equal(out.status, 401);
  assert.equal((await out.json()).error.code, "not_logged_in");
  assert.equal(made.length, 0);

  const res = await showcaseKeyReply("mine", {demoKey: DEMO, uid: "u1", mint});
  assert.equal(res.status, 200);
  assert.deepEqual(await res.json(), {
    key: "ag_sk_live_for_u1",
    expires: 1234,
  });
  assert.equal(res.headers.get("cache-control"), "no-store");
  assert.deepEqual(made, ["u1"]);
});

test("anything else isn't a source", async () => {
  const {made, mint} = minter();
  for (const source of [undefined, "", "theirs", 1]) {
    const res = await showcaseKeyReply(source, {demoKey: DEMO, uid: "u", mint});
    assert.equal(res.status, 400);
  }
  assert.equal(made.length, 0);
});
