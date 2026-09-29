// Unit tests for lib/app-version.ts:  npm run test:unit

import assert from "node:assert/strict";
import {test} from "node:test";

const {shouldReload} = await import("../lib/app-version.ts");

const page = {current: "dpl_old", reloadedFor: null, typing: false};

test("a page reloads when a newer build is live", () => {
  assert.equal(shouldReload({...page, live: "dpl_new"}), true);
});

test("the live build's page doesn't reload", () => {
  assert.equal(shouldReload({...page, live: "dpl_old"}), false);
});

test("nothing to compare, nothing done", () => {
  assert.equal(shouldReload({...page, live: null}), false);
  assert.equal(shouldReload({...page, current: "", live: "dpl_new"}), false);
});

test("never twice for the same build, so it can't loop", () => {
  assert.equal(
    shouldReload({...page, live: "dpl_new", reloadedFor: "dpl_new"}),
    false,
  );
  // A build newer still reloads again.
  assert.equal(
    shouldReload({...page, live: "dpl_newer", reloadedFor: "dpl_new"}),
    true,
  );
});

test("not while something's typed, which a reload would lose", () => {
  assert.equal(shouldReload({...page, live: "dpl_new", typing: true}), false);
});
