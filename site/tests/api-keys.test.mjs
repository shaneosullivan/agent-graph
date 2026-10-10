// lib/api/keys.ts's browser keys (the API showcase's, for a browser that's
// logged in) against the Firestore emulator, directly:  npm run test:ci

import assert from "node:assert/strict";
import {randomBytes} from "node:crypto";
import {register} from "node:module";
import {test} from "node:test";

register("../scripts/resolve-ts.mjs", import.meta.url);

const skip =
  !process.env.FIRESTORE_EMULATOR_HOST && "needs the Firestore emulator";
const HOUR = 60 * 60 * 1000;
const someone = () => `u-${randomBytes(6).toString("hex")}`;

test(
  "a browser key reads as its account for an hour, then stops",
  {skip},
  async () => {
    const keys = await import("../lib/api/keys.ts");
    const uid = someone();
    const {key, expires} = await keys.newBrowserKey(uid);
    assert.ok(Math.abs(expires - (Date.now() + HOUR)) < 60_000);
    const now = await keys.keyOf(key);
    assert.equal(now?.uid, uid);
    assert.equal(now?.kind, "secret");
    assert.equal(now?.graphs, null);

    const old = await keys.newBrowserKey(uid, Date.now() - 2 * HOUR);
    assert.equal(await keys.keyOf(old.key), null, "run out");
  },
);

test(
  "browser keys aren't listed, nor counted against the account's keys, and don't pile up",
  {skip, timeout: 60_000},
  async () => {
    const keys = await import("../lib/api/keys.ts");
    const {firestore} = await import("../lib/firebase.ts");
    const uid = someone();
    for (let i = 0; i < 25; i++) {
      await keys.newBrowserKey(uid);
    }
    const kept = await firestore()
      .collection("api-keys")
      .where("uid", "==", uid)
      .get();
    assert.equal(kept.size, 20, "the oldest go");
    assert.deepEqual(await keys.keysOf(uid), []);

    for (let i = 0; i < keys.MOST_API_KEYS; i++) {
      assert.ok(
        await keys.newKey(uid, {name: `k${i}`, kind: "secret", graphs: null}),
        `key ${i}`,
      );
    }
    assert.equal(
      await keys.newKey(uid, {name: "one more", kind: "secret", graphs: null}),
      null,
    );
    assert.equal((await keys.keysOf(uid)).length, keys.MOST_API_KEYS);

    await keys.deleteKeysOf(uid);
    assert.equal(
      (await firestore().collection("api-keys").where("uid", "==", uid).get())
        .size,
      0,
      "an account's deletion takes its browser keys too",
    );
  },
);
