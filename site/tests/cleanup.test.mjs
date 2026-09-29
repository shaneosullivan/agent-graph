// lib/cleanup.ts (the daily cron that deletes logs with no event for a
// week) against the Firestore emulator:  npm run test:ci

import assert from "node:assert/strict";
import {randomBytes} from "node:crypto";
import {register} from "node:module";
import {test} from "node:test";

register("../scripts/resolve-ts.mjs", import.meta.url);

const skip =
  !process.env.FIRESTORE_EMULATOR_HOST && "needs the Firestore emulator";
const DAY = 24 * 60 * 60 * 1000;

const newId = () =>
  Array.from(
    randomBytes(12),
    b =>
      "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"[b % 62],
  ).join("");

async function setup() {
  const store = await import("../lib/store.ts");
  const cleanup = await import("../lib/cleanup.ts");
  const {firestore} = await import("../lib/firebase.ts");
  const {storageId} = await import("../lib/encryption.ts");
  const {Timestamp, FieldValue} = await import("firebase-admin/firestore");
  const doc = id => firestore().collection("logs").doc(storageId(id));
  const state = firestore().collection("cleanup").doc("state");
  /** A log whose chunks were written `ages` days ago (null: before times were recorded). */
  async function log(ages, createdDaysAgo = 0) {
    const id = newId();
    await store.createLog(id, {source: "watch"}, "0\n");
    for (let i = 1; i < ages.length; i++)
      await store.appendChunk(id, 2 * i, `${i}\n`);
    const chunks = (
      await doc(id).collection("chunks").orderBy("__name__").get()
    ).docs;
    await Promise.all(
      chunks.map((chunk, i) =>
        chunk.ref.update({
          t:
            ages[i] === null
              ? FieldValue.delete()
              : Timestamp.fromMillis(Date.now() - ages[i] * DAY),
        }),
      ),
    );
    if (createdDaysAgo)
      await doc(id).update({
        createdAt: Timestamp.fromMillis(Date.now() - createdDaysAgo * DAY),
      });
    return id;
  }
  const exists = async id => (await doc(id).get()).exists;
  const chunkCount = async id =>
    (await doc(id).collection("chunks").get()).size;
  return {store, cleanup, state, log, exists, chunkCount, Timestamp};
}

test(
  "logs with no event for a week are deleted, with their chunks, and the rest kept",
  {skip, timeout: 60_000},
  async () => {
    const {cleanup, state, log, exists, chunkCount, store, Timestamp} =
      await setup();
    // Times have been recorded for a month.
    await state.set({since: Timestamp.fromMillis(Date.now() - 30 * DAY)});
    const idle = await log([9, 8]);
    const active = await log([20, 6]); // the newest chunk counts
    const recorded = await log([null, 1]); // an older chunk from before times were recorded
    const old = await log([null, null], 30);
    const young = await log([null], 2); // before times: counts from when it was made

    const result = await cleanup.deleteIdleLogs({budgetMs: 30_000});
    assert.equal(result.done, true);
    assert.ok(
      result.deleted >= 2 && result.checked >= 5,
      JSON.stringify(result),
    );
    for (const id of [idle, old]) {
      assert.equal(await exists(id), false);
      assert.equal(await chunkCount(id), 0, "and its chunks");
      assert.equal(await store.getMeta(id), null, "the site doesn't know it");
    }
    for (const id of [active, recorded, young])
      assert.equal(await exists(id), true);
  },
);

test(
  "before times were recorded, a log counts as active from the cron's first run",
  {skip, timeout: 60_000},
  async () => {
    const {cleanup, state, log, exists} = await setup();
    await state.delete();
    const old = await log([null], 30);
    await cleanup.deleteIdleLogs({budgetMs: 30_000});
    assert.equal(await exists(old), true, "kept for now");
    const since = (await state.get()).get("since").toMillis();
    assert.ok(Math.abs(since - Date.now()) < 60_000, "since the first run");
    // A week on (and a day), it goes.
    await cleanup.deleteIdleLogs({budgetMs: 30_000, now: Date.now() + 8 * DAY});
    assert.equal(await exists(old), false);
    assert.equal(
      (await state.get()).get("since").toMillis(),
      since,
      "the first run's, still",
    );
  },
);

test(
  "a run out of time carries on where it stopped",
  {skip, timeout: 120_000},
  async () => {
    const {cleanup, state, log, exists, Timestamp} = await setup();
    await state.set({since: Timestamp.fromMillis(Date.now() - 30 * DAY)});
    const idle = [];
    for (let i = 0; i < 12; i++) idle.push(await log([10]));
    // With no time, a few, and the next run carries on after them.
    const first = await cleanup.deleteIdleLogs({budgetMs: -1, page: 5});
    const cursor = (await state.get()).get("cursor");
    assert.equal(first.done, false);
    assert.equal(first.checked, 5, JSON.stringify(first));
    assert.equal(typeof cursor, "string");
    let runs = 0;
    for (let result = first; !result.done; runs++) {
      assert.ok(runs < 100, "it ends");
      result = await cleanup.deleteIdleLogs({budgetMs: 30_000, page: 5});
    }
    for (const id of idle) assert.equal(await exists(id), false);
    assert.equal(
      (await state.get()).get("cursor"),
      undefined,
      "done: from the start next time",
    );
  },
);

// A share left running while it's deleted (a week without events, say):
// what it sends then is refused, so it stops and says so, rather than
// leaving chunks nothing will ever delete.
test(
  "a deleted log can't be added to or trimmed",
  {skip, timeout: 60_000},
  async () => {
    const {cleanup, state, log, exists, store, Timestamp} = await setup();
    const {firestore} = await import("../lib/firebase.ts");
    const {storageId} = await import("../lib/encryption.ts");
    await state.set({since: Timestamp.fromMillis(Date.now() - 30 * DAY)});
    const id = await log([9]);
    // Guesses at its password were counted.
    const guesses = firestore()
      .collection("unlock-attempts")
      .doc(`log-${storageId(id)}`);
    await guesses.set({n: 1});
    assert.ok(await store.getMeta(id), "known, and cached");
    await cleanup.deleteIdleLogs({budgetMs: 30_000});
    assert.equal(await exists(id), false);
    assert.equal(await store.getMeta(id), null, "forgotten here too");
    assert.equal((await guesses.get()).exists, false, "and the guesses at it");
    await assert.rejects(store.appendChunk(id, 2, "1\n"), store.LogGone);
    await assert.rejects(store.trimLog(id, 2), store.LogGone);
    const chunks = await firestore()
      .collection("logs")
      .doc(storageId(id))
      .collection("chunks")
      .get();
    assert.equal(chunks.size, 0, "nothing left behind");
  },
);

// A run stops when its time's up, after the logs it's looking at at once,
// not a whole page.
test(
  "a run's time is checked as it goes",
  {skip, timeout: 120_000},
  async () => {
    const {cleanup, state, log, Timestamp} = await setup();
    await state.set({since: Timestamp.fromMillis(Date.now() - 30 * DAY)});
    for (let i = 0; i < 25; i++) await log([0]);
    const first = await cleanup.deleteIdleLogs({budgetMs: -1, page: 100});
    assert.deepEqual([first.checked, first.done], [10, false]);
  },
);

// Whether a log is idle is judged again as it's deleted, with its newest
// chunk, so one written to since it was first judged (a share quiet for a
// week, resumed) is kept, with what was added.
test(
  "a log written to after it's judged idle is kept",
  {skip, timeout: 60_000},
  async () => {
    const {cleanup, state, log, exists, store, Timestamp} = await setup();
    const {storageId} = await import("../lib/encryption.ts");
    await state.set({since: Timestamp.fromMillis(Date.now() - 30 * DAY)});
    const id = await log([9]);
    await store.appendChunk(id, 2, "1\n");
    assert.equal(
      await cleanup.deleteIfIdle(
        storageId(id),
        Date.now() - 30 * DAY,
        Date.now(),
      ),
      false,
    );
    assert.equal(await exists(id), true);
    assert.equal((await store.readChunks(id, "")).text, "0\n1\n");
  },
);

// A deletion that stopped partway (the run ran out of time) leaves the log
// marked: nothing more can be added, it can't be read, and the next run
// finishes it, so nothing's left behind.
test(
  "a deletion that stopped partway is finished",
  {skip, timeout: 60_000},
  async () => {
    const {cleanup, state, log, exists, chunkCount, store, Timestamp} =
      await setup();
    const {firestore} = await import("../lib/firebase.ts");
    const {storageId} = await import("../lib/encryption.ts");
    await state.set({since: Timestamp.fromMillis(Date.now() - 30 * DAY)});
    const id = await log([0, 0]);
    await firestore()
      .collection("logs")
      .doc(storageId(id))
      .update({deleting: true});
    await assert.rejects(store.appendChunk(id, 4, "2\n"), store.LogGone);
    await assert.rejects(store.trimLog(id, 2), store.LogGone);
    assert.equal(
      await store.getMeta(id, Date.now() + store.META_CACHE_MS + 1),
      null,
    );
    await cleanup.deleteIdleLogs({budgetMs: 30_000});
    assert.equal(await exists(id), false);
    assert.equal(await chunkCount(id), 0);
  },
);

// Logs stored before storage ids (README's migration moves them) are
// judged the same way, with what's added to them since, which is stored
// under their storage id; and deleted with it.
test(
  "logs from before storage ids are deleted the same way",
  {skip, timeout: 60_000},
  async () => {
    const {cleanup, state, store, Timestamp} = await setup();
    const {firestore} = await import("../lib/firebase.ts");
    const {encryptChunk, storageId} = await import("../lib/encryption.ts");
    await state.set({since: Timestamp.fromMillis(Date.now() - 30 * DAY)});
    const id = newId();
    const old = firestore().collection("logs").doc(id);
    const added = firestore()
      .collection("logs")
      .doc(storageId(id))
      .collection("chunks");
    await old.set({
      source: "watch",
      createdAt: Timestamp.fromMillis(Date.now() - 30 * DAY),
    });
    await old
      .collection("chunks")
      .doc(store.chunkKey(0))
      .set({e: encryptChunk(id, store.chunkKey(0), "0\n")});
    try {
      // Written to today: kept, and can be trimmed.
      await store.appendChunk(id, 2, "1\n");
      await store.appendChunk(id, 4, "2\n");
      await cleanup.deleteIdleLogs({budgetMs: 30_000});
      assert.equal((await old.get()).exists, true);
      assert.equal(await store.trimLog(id, 4), 1);
      // A week on (and a day), it goes, and what was added to it.
      await cleanup.deleteIdleLogs({
        budgetMs: 30_000,
        now: Date.now() + 8 * DAY,
      });
      assert.equal((await old.get()).exists, false);
      assert.equal((await old.collection("chunks").get()).size, 0);
      assert.equal((await added.get()).size, 0, "nothing left behind");
      await assert.rejects(store.appendChunk(id, 6, "3\n"), store.LogGone);
    } finally {
      // (So the migration's tests don't find it.)
      await firestore().recursiveDelete(old);
      await firestore().recursiveDelete(added.parent);
    }
  },
);

// The migration copies logs, then deletes the old copies (a separate step):
// if that's left for a while, the copy goes with its old copy, not before,
// when the old one would still take appends no one could read.
test(
  "a log the migration copied goes with its old copy",
  {skip, timeout: 60_000},
  async () => {
    const {cleanup, state, store, exists, Timestamp} = await setup();
    const {firestore} = await import("../lib/firebase.ts");
    const {encryptChunk, storageId} = await import("../lib/encryption.ts");
    await state.set({since: Timestamp.fromMillis(Date.now() - 30 * DAY)});
    const id = await (async () => {
      const id = newId();
      await store.createLog(id, {source: "watch"}, "0\n");
      return id;
    })();
    const old = firestore().collection("logs").doc(id);
    try {
      await old.set({
        source: "watch",
        createdAt: Timestamp.fromMillis(Date.now() - 30 * DAY),
      });
      await old
        .collection("chunks")
        .doc(store.chunkKey(0))
        .set({e: encryptChunk(id, store.chunkKey(0), "0\n")});
      await firestore()
        .collection("logs")
        .doc(storageId(id))
        .update({oldCopy: true});
      await store.appendChunk(id, 2, "1\n");
      await cleanup.deleteIdleLogs({budgetMs: 30_000});
      assert.equal(await exists(id), true);
      assert.equal((await old.get()).exists, true);
      // A week on, the copy alone isn't deleted, and takes what's added.
      assert.equal(
        await cleanup.deleteIfIdle(
          storageId(id),
          Date.now(),
          Date.now() + 8 * DAY,
        ),
        false,
      );
      assert.equal(await exists(id), true);
      await store.appendChunk(id, 4, "2\n");
      assert.equal((await store.readChunks(id, "")).text, "0\n1\n2\n");
      await cleanup.deleteIdleLogs({
        budgetMs: 30_000,
        now: Date.now() + 8 * DAY,
      });
      assert.equal(await exists(id), false);
      assert.equal((await old.get()).exists, false);
      await assert.rejects(store.appendChunk(id, 6, "3\n"), store.LogGone);
    } finally {
      await firestore().recursiveDelete(old);
    }
  },
);

// One from before storage ids that's being deleted is gone too.
test(
  "a log from before storage ids that's being deleted can't be added to",
  {skip, timeout: 60_000},
  async () => {
    const {store, Timestamp} = await setup();
    const {firestore} = await import("../lib/firebase.ts");
    const id = newId();
    const old = firestore().collection("logs").doc(id);
    try {
      await old.set({
        source: "watch",
        createdAt: Timestamp.now(),
        deleting: true,
      });
      await assert.rejects(store.appendChunk(id, 2, "1\n"), store.LogGone);
      await assert.rejects(store.trimLog(id, 2), store.LogGone);
    } finally {
      await firestore().recursiveDelete(old);
    }
  },
);

// Deleting a log with its copy marks both first, so if it stops partway,
// neither takes what's sent until the next run finishes it.
test(
  "a log and its copy stopped partway through deletion are both marked",
  {skip, timeout: 60_000},
  async () => {
    const {cleanup, state, store, exists, Timestamp} = await setup();
    const {firestore} = await import("../lib/firebase.ts");
    const {encryptChunk, storageId} = await import("../lib/encryption.ts");
    await state.set({since: Timestamp.fromMillis(Date.now() - 30 * DAY)});
    const id = newId();
    await store.createLog(id, {source: "watch"}, "0\n");
    await firestore()
      .collection("logs")
      .doc(storageId(id))
      .update({oldCopy: true});
    const old = firestore().collection("logs").doc(id);
    const db = firestore();
    const recursiveDelete = db.recursiveDelete;
    try {
      await old.set({
        source: "watch",
        createdAt: Timestamp.fromMillis(Date.now() - 30 * DAY),
      });
      await old
        .collection("chunks")
        .doc(store.chunkKey(0))
        .set({e: encryptChunk(id, store.chunkKey(0), "0\n")});
      db.recursiveDelete = async () => {
        throw new Error("stopped");
      };
      await assert.rejects(
        cleanup.deleteIfIdle(id, Date.now(), Date.now() + 8 * DAY),
        /stopped/,
      );
      db.recursiveDelete = recursiveDelete;
      await assert.rejects(store.appendChunk(id, 2, "1\n"), store.LogGone);
      await cleanup.deleteIdleLogs({budgetMs: 30_000});
      assert.equal(await exists(id), false);
      assert.equal((await old.get()).exists, false);
    } finally {
      db.recursiveDelete = recursiveDelete;
      await firestore().recursiveDelete(old);
    }
  },
);
