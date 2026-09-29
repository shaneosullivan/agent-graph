// lib/store.ts against the Firestore emulator, directly:  npm run test:ci
// (which runs it inside the emulator, after the API tests).

import assert from "node:assert/strict";
import {randomBytes} from "node:crypto";
import {register} from "node:module";
import {test} from "node:test";

register("../scripts/resolve-ts.mjs", import.meta.url);

const skip =
  !process.env.FIRESTORE_EMULATOR_HOST && "needs the Firestore emulator";

/** Records every query's limit and every chunk fetched while `fn` runs. */
async function watchQueries(fn) {
  const {Query} = await import("firebase-admin/firestore");
  const seen = {limits: [], fetched: 0};
  const {limit, get, stream} = Query.prototype;
  Query.prototype.limit = function (n) {
    seen.limits.push(n);
    return limit.call(this, n);
  };
  Query.prototype.get = async function () {
    const snap = await get.call(this);
    seen.fetched += snap.size;
    return snap;
  };
  Query.prototype.stream = function () {
    throw new Error(
      "a stream isn't cancelled by leaving it, so it isn't bounded",
    );
  };
  try {
    return {result: await fn(), ...seen};
  } finally {
    Object.assign(Query.prototype, {limit, get, stream});
  }
}

/** Every page after `first`, joined; failing, rather than hanging, if they never end. */
async function rest(store, id, first, most) {
  let text = "";
  let page = first;
  for (let pages = 0; page.more; pages++) {
    assert.ok(pages < most, "reading never ended");
    page = await store.readChunks(id, page.last);
    text += page.text;
  }
  return text;
}

const newId = () =>
  Array.from(
    randomBytes(12),
    b =>
      "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"[b % 62],
  ).join("");

// R17: a read holds a bounded number of chunks at once, and fetches a
// bounded number beyond those it uses, however big the chunks are. (Not
// just the response: the fetching is what passes through memory.)
test(
  "a read fetches chunks a few at a time",
  {skip, timeout: 60_000},
  async () => {
    const {BYTES_PER_READ, CHUNKS_PER_QUERY, MAX_CHUNK_BYTES} =
      await import("../lib/config.ts");
    const store = await import("../lib/store.ts");

    const id = newId();
    const big = `${"x".repeat(MAX_CHUNK_BYTES - 1)}\n`;
    await store.createLog(id, {source: "watch"}, big);
    const count = CHUNKS_PER_QUERY + 8;
    for (let i = 1; i < count; i++)
      await store.appendChunk(id, i * MAX_CHUNK_BYTES, big);

    const {
      result: read,
      limits,
      fetched,
    } = await watchQueries(() => store.readChunks(id, ""));
    assert.ok(read.more);
    const used = Buffer.byteLength(read.text) / MAX_CHUNK_BYTES;
    assert.equal(used, Math.ceil(BYTES_PER_READ / MAX_CHUNK_BYTES));
    assert.ok(
      limits.length > 0 && limits.every(n => n <= CHUNKS_PER_QUERY),
      `limits: ${limits}`,
    );
    assert.ok(
      fetched < used + CHUNKS_PER_QUERY,
      `${fetched} fetched for ${used} used`,
    );

    // And paging from there gets the rest, in order.
    assert.equal(
      read.text + (await rest(store, id, read, count)),
      big.repeat(count),
    );
  },
);

// R17: a read of many small chunks takes several queries, each carrying on
// from the last, and stops at CHUNKS_PER_READ, saying there's more.
test(
  "a read of many small chunks carries on across queries",
  {skip, timeout: 120_000},
  async () => {
    const {CHUNKS_PER_QUERY, CHUNKS_PER_READ} =
      await import("../lib/config.ts");
    const store = await import("../lib/store.ts");

    const id = newId();
    const count = CHUNKS_PER_READ + 50;
    const chunk = i => `{"n":${i}}\n`;
    await store.createLog(id, {source: "watch"}, chunk(0));
    let offset = Buffer.byteLength(chunk(0));
    for (let i = 1; i < count; i++) {
      await store.appendChunk(id, offset, chunk(i));
      offset += Buffer.byteLength(chunk(i));
    }
    const all = Array.from({length: count}, (_, i) => chunk(i)).join("");
    const upTo = n => Array.from({length: n}, (_, i) => chunk(i)).join("");

    const {result: first, limits} = await watchQueries(() =>
      store.readChunks(id, ""),
    );
    assert.equal(
      first.text,
      upTo(CHUNKS_PER_READ),
      "each chunk once, in order",
    );
    assert.ok(first.more, "says there's more");
    assert.equal(limits.length, Math.ceil(CHUNKS_PER_READ / CHUNKS_PER_QUERY));
    assert.ok(
      limits.every(n => n <= CHUNKS_PER_QUERY),
      `limits: ${limits}`,
    );

    const second = await store.readChunks(id, first.last);
    assert.equal(first.text + second.text, all);
    assert.equal(second.more, false);
  },
);

/** The log documents `fn` creates, found by listing them before and after. */
async function newDocs(fn) {
  const {firestore} = await import("../lib/firebase.ts");
  const names = async () =>
    new Set(
      (await firestore().collection("logs").listDocuments()).map(d => d.id),
    );
  const before = await names();
  await fn();
  return (await firestore().collection("logs").listDocuments()).filter(
    d => !before.has(d.id),
  );
}

// R18: the database doesn't hold the ids, which are the links that open
// logs, and only holds ciphertext.
test(
  "a log isn't stored under its id, and its chunks are ciphertext",
  {skip, timeout: 60_000},
  async () => {
    const store = await import("../lib/store.ts");
    const id = newId();
    const marker = "a very recognisable summary 7c1f";
    const text = `{"summary":"${marker}"}\n`;
    const [doc, ...others] = await newDocs(() =>
      store.createLog(id, {source: "watch"}, text),
    );
    assert.equal(others.length, 0);
    assert.ok(!doc.id.includes(id), `stored under ${doc.id}`);
    assert.ok(
      !doc.id.includes(id.toLowerCase()) && !doc.id.includes(id.toUpperCase()),
    );
    await store.appendChunk(id, Buffer.byteLength(text), text);

    const stored = await doc.get();
    assert.deepEqual(Object.keys(stored.data()).sort(), [
      "createdAt",
      "mac",
      "source",
    ]);
    const chunks = await doc.collection("chunks").get();
    assert.equal(chunks.size, 2);
    for (const chunk of chunks.docs) {
      assert.deepEqual(
        Object.keys(chunk.data()).sort(),
        ["e", "end", "n", "t"],
        "ciphertext, its length and where it ends, and when it was written",
      );
      assert.ok(Math.abs(chunk.get("t").toMillis() - Date.now()) < 60_000);
      assert.ok(
        !Buffer.from(chunk.get("e")).includes(Buffer.from("recognisable")),
      );
    }
    assert.equal(
      (await store.readChunks(id, "")).text,
      text + text,
      "the site still reads it",
    );
  },
);

// Keyframes: a live share keeps only its last two keyframes' worth, so it
// asks for the chunks before one to be deleted.
test(
  "trimming deletes the chunks before an offset, and nothing from it on",
  {skip, timeout: 60_000},
  async () => {
    const store = await import("../lib/store.ts");
    const id = newId();
    const part = n => `{"n":${n}}\n`;
    await store.createLog(id, {source: "watch"}, part(0));
    const offsets = [0];
    let offset = Buffer.byteLength(part(0));
    for (let i = 1; i < 6; i++) {
      offsets.push(offset);
      await store.appendChunk(id, offset, part(i));
      offset += Buffer.byteLength(part(i));
    }
    assert.equal(await store.firstChunkOffset(id), 0);

    assert.equal(await store.trimLog(id, offsets[3]), 3);
    assert.equal(await store.firstChunkOffset(id), offsets[3]);
    const page = await store.readChunks(id, "");
    assert.equal(page.text, part(3) + part(4) + part(5));
    assert.equal(
      page.first,
      store.chunkKey(offsets[3]),
      "where what's read starts",
    );
    // Nothing more before it, and appending carries on.
    assert.equal(await store.trimLog(id, offsets[2]), 0);
    await store.appendChunk(id, offset, part(6));
    assert.equal(
      (await store.readChunks(id, "")).text,
      part(3) + part(4) + part(5) + part(6),
    );
    // All of it.
    assert.equal(await store.trimLog(id, offset + 100), 4);
    assert.equal(await store.firstChunkOffset(id), null);
  },
);

test(
  "trimming more chunks than a batch holds deletes them all",
  {skip, timeout: 120_000},
  async () => {
    const store = await import("../lib/store.ts");
    const id = newId();
    await store.createLog(id, {source: "watch"}, "");
    const count = 620;
    // (One at a time: each follows on from the one before.)
    for (let i = 0; i < count; i++)
      await store.appendChunk(id, i * 10, `${String(i).padStart(9, "0")}\n`);
    // Newest first: a reader starting from the first chunk left never starts
    // partway through what's being deleted.
    const {Transaction} = await import("firebase-admin/firestore");
    const {delete: remove} = Transaction.prototype;
    const batches = [];
    Transaction.prototype.delete = function (ref) {
      if (!batches.includes(this.keys)) batches.push((this.keys = []));
      this.keys.push(ref.id);
      return remove.call(this, ref);
    };
    try {
      assert.equal(await store.trimLog(id, 600 * 10), 600);
    } finally {
      Object.assign(Transaction.prototype, {delete: remove});
    }
    const key = i => store.chunkKey(i * 10);
    assert.deepEqual(
      batches.map(keys => [keys.length, [...keys].sort()[0]]),
      [
        [500, key(100)],
        [100, key(0)],
      ],
    );
    assert.equal(await store.firstChunkOffset(id), 6000);
    assert.equal((await store.readChunks(id, "")).first, store.chunkKey(6000));
  },
);

// R44: each chunk starts where the one before it ends, so chunks never
// overlap, and what a log stores is the span from its first to its last
// (which the append route bounds). A retry of what's stored is accepted.
test(
  "a chunk follows on from the one before",
  {skip, timeout: 60_000},
  async () => {
    const store = await import("../lib/store.ts");
    const misplaced = promise => assert.rejects(promise, store.Misplaced);

    const id = newId();
    await store.createLog(id, {source: "watch"}, "AA\n");
    await store.appendChunk(id, 3, "BBBB\n");
    await store.appendChunk(id, 3, "BBBB\n");
    await misplaced(store.appendChunk(id, 4, "CCCCCC\n"));
    await misplaced(store.appendChunk(id, 9, "CCCCCC\n"));
    await misplaced(store.appendChunk(id, 1, "C\n"));
    await store.appendChunk(id, 8, "CCCCCC\n");
    await store.appendChunk(id, 0, "AA\n");
    assert.equal((await store.readChunks(id, "")).text, "AA\nBBBB\nCCCCCC\n");

    // Trimmed: nothing goes before the first chunk kept (at 0, say), but it
    // can be sent again.
    await store.trimLog(id, 3);
    await misplaced(store.appendChunk(id, 0, "AA\n"));
    await store.appendChunk(id, 3, "BBBB\n");
    await store.appendChunk(id, 15, "D\n");
    assert.equal((await store.readChunks(id, "")).text, "BBBB\nCCCCCC\nD\n");

    // A log made empty starts at 0.
    const empty = newId();
    await store.createLog(empty, {source: "watch"}, "");
    await misplaced(store.appendChunk(empty, 5, "A\n"));
    await store.appendChunk(empty, 0, "A\n");
  },
);

// Chunks are stored one after another; a gap means the log's start was
// trimmed while it was read, so a read stops there, and the viewer, seeing
// the next read start somewhere else, starts again.
test("a read stops at a gap", {skip, timeout: 60_000}, async () => {
  const store = await import("../lib/store.ts");
  const {firestore} = await import("../lib/firebase.ts");
  const {storageId} = await import("../lib/encryption.ts");
  const id = newId();
  await store.createLog(id, {source: "watch"}, "A\n");
  await store.appendChunk(id, 2, "B\n");
  await store.appendChunk(id, 4, "X\n");
  await store.appendChunk(id, 6, "C\n");
  // As a trim would leave it, had it deleted up to 6 as this was read.
  await firestore()
    .collection("logs")
    .doc(storageId(id))
    .collection("chunks")
    .doc(store.chunkKey(4))
    .delete();
  const page = await store.readChunks(id, "");
  assert.deepEqual(page, {
    text: "A\nB\n",
    first: store.chunkKey(0),
    last: store.chunkKey(2),
    more: true,
  });
  const next = await store.readChunks(id, page.last);
  assert.deepEqual(next, {
    text: "C\n",
    first: store.chunkKey(6),
    last: store.chunkKey(6),
    more: false,
  });
});

// R18: metadata changed in the database (a password removed, or another
// log's metadata copied over) is refused, not trusted.
test(
  "metadata changed in the database is refused",
  {skip, timeout: 60_000},
  async () => {
    const {FieldValue} = await import("firebase-admin/firestore");
    const {hashPassword} = await import("../lib/crypto.ts");
    const store = await import("../lib/store.ts");

    const pw = await hashPassword("pässwörd");
    const [protectedId, openId, keptId] = [newId(), newId(), newId()];
    const [protectedDoc] = await newDocs(() =>
      store.createLog(protectedId, {source: "watch", pw}, ""),
    );
    const [openDoc] = await newDocs(() =>
      store.createLog(openId, {source: "paste"}, ""),
    );
    await store.createLog(keptId, {source: "upload", pw}, "");

    const kept = await store.getMeta(keptId);
    assert.equal(kept?.pw, pw, "untouched metadata is read as stored");
    assert.equal(kept?.source, "upload");

    await protectedDoc.update({pw: FieldValue.delete()});
    assert.equal(await store.getMeta(protectedId), null, "password removed");

    // A pasted log made to look live (the viewer then judges it by the clock).
    const [pastedId] = [newId()];
    const [pastedDoc] = await newDocs(() =>
      store.createLog(pastedId, {source: "paste"}, ""),
    );
    await pastedDoc.update({source: "watch"});
    assert.equal(await store.getMeta(pastedId), null, "source changed");

    // Another log's metadata, whole (its own tag included), copied over.
    const [copyId] = [newId()];
    const [copyDoc] = await newDocs(() =>
      store.createLog(copyId, {source: "watch", pw}, ""),
    );
    await copyDoc.set((await openDoc.get()).data());
    assert.equal(await store.getMeta(copyId), null, "another log's metadata");
  },
);

// ---- R18: the migration of logs stored the old way (under their ids) ----

/** A log stored the old way. `encryptAs` another id makes it one that won't decrypt. */
async function oldLog({
  meta = {source: "watch"},
  chunks = ['{"n":1}\n'],
  parent = true,
  encryptAs,
} = {}) {
  const {Timestamp} = await import("firebase-admin/firestore");
  const {encryptChunk} = await import("../lib/encryption.ts");
  const {firestore} = await import("../lib/firebase.ts");
  const {chunkKey} = await import("../lib/store.ts");
  const id = newId();
  const doc = firestore().collection("logs").doc(id);
  if (parent) await doc.set({...meta, createdAt: Timestamp.now()});
  let offset = 0;
  for (const text of chunks) {
    const key = chunkKey(offset);
    await doc
      .collection("chunks")
      .doc(key)
      .set({e: encryptChunk(encryptAs ?? id, key, text)});
    offset += Buffer.byteLength(text);
  }
  return {id, doc, meta, chunks, offset};
}

/** Runs the migration script; `env` values of undefined are removed. */
async function migrate(args = [], env = {}) {
  const {spawnSync} = await import("node:child_process");
  const environment = {...process.env, ...env};
  for (const [k, v] of Object.entries(env))
    if (v === undefined) delete environment[k];
  return spawnSync(
    process.execPath,
    [
      "--experimental-strip-types",
      "--no-warnings",
      "scripts/migrate-storage-ids.mjs",
      ...args,
    ],
    {cwd: new URL("..", import.meta.url), encoding: "utf8", env: environment},
  );
}

/** Log `id`'s copy's metadata document. */
async function copyOf(id) {
  const {storageId} = await import("../lib/encryption.ts");
  const {firestore} = await import("../lib/firebase.ts");
  return firestore().collection("logs").doc(storageId(id)).get();
}

async function gone(doc) {
  return (
    !(await doc.get()).exists &&
    (await doc.collection("chunks").listDocuments()).length === 0
  );
}

test(
  "the migration copies logs, then deletes the old copies",
  {skip, timeout: 60_000},
  async () => {
    const {hashPassword} = await import("../lib/crypto.ts");
    const store = await import("../lib/store.ts");
    const pw = await hashPassword("pässwörd");
    const a = await oldLog({
      meta: {source: "watch", pw},
      chunks: ['{"n":1}\n', '{"n":2}\n'],
    });
    const b = await oldLog({meta: {source: "paste"}, chunks: ['{"n":3}\n']});
    // Appended by the new site before the copy (a live log across the deploy).
    const late = '{"n":"late"}\n';
    await store.appendChunk(a.id, a.offset, late);
    assert.equal(
      await store.getMeta(a.id),
      null,
      "not found before it's copied",
    );

    const copied = await migrate();
    assert.equal(copied.status, 0, copied.stderr);
    assert.match(copied.stdout, /Copied 2 logs\./);
    for (const log of [a, b]) {
      const meta = await store.getMeta(log.id);
      assert.equal(meta?.source, log.meta.source);
      assert.equal(meta?.pw, log.meta.pw);
      assert.ok((await log.doc.get()).exists, "copying deletes nothing");
      assert.equal(
        (await copyOf(log.id)).get("oldCopy"),
        true,
        "marked as having an old copy",
      );
    }
    assert.equal(
      (await store.readChunks(a.id, "")).text,
      a.chunks.join("") + late,
    );
    assert.equal((await store.readChunks(b.id, "")).text, b.chunks.join(""));
    assert.equal((await migrate()).status, 0, "copying again is fine");

    // Since the copy, the old site stored another chunk of b, and a's
    // metadata didn't make it: neither is deleted until it's copied again.
    const {encryptChunk, storageId} = await import("../lib/encryption.ts");
    const {firestore} = await import("../lib/firebase.ts");
    const more = '{"n":4}\n';
    const key = store.chunkKey(b.offset);
    await b.doc
      .collection("chunks")
      .doc(key)
      .set({e: encryptChunk(b.id, key, more)});
    await firestore()
      .collection("logs")
      .doc(storageId(a.id))
      .update({mac: "lost"});
    const early = await migrate(["--delete-old"]);
    assert.equal(early.status, 1);
    assert.match(
      early.stderr,
      new RegExp(`logs/${a.id}: its metadata hasn't been copied`),
    );
    assert.match(
      early.stderr,
      new RegExp(`logs/${b.id}: chunk ${key} hasn't been copied`),
    );
    for (const log of [a, b])
      assert.ok((await log.doc.get()).exists && !(await gone(log.doc)), "kept");

    assert.equal((await migrate()).status, 0);
    const deleted = await migrate(["--delete-old"]);
    assert.equal(deleted.status, 0, deleted.stderr);
    assert.match(deleted.stdout, /Deleted the old copies of 2 logs\./);
    for (const log of [a, b]) {
      assert.ok(await gone(log.doc), "the old copy is gone");
      assert.equal(
        (await copyOf(log.id)).get("oldCopy"),
        undefined,
        "and its mark",
      );
    }
    assert.equal(
      (await store.readChunks(a.id, "")).text,
      a.chunks.join("") + late,
      "and it still reads the same",
    );
    assert.equal(
      (await store.readChunks(b.id, "")).text,
      b.chunks.join("") + more,
    );
    assert.match(
      (await migrate(["--delete-old"])).stdout,
      /Deleted the old copies of 0 logs\./,
    );
  },
);

// R44: once copied, a log's chunks all say where they end (what the old
// site stored didn't), and appends follow on from them.
test("copied chunks say where they end", {skip, timeout: 60_000}, async () => {
  const store = await import("../lib/store.ts");
  const {firestore} = await import("../lib/firebase.ts");
  const {storageId} = await import("../lib/encryption.ts");
  const a = await oldLog();
  const big = c => `${c.repeat(10 * 1024 - 1)}\n`;
  let offset = a.offset;
  for (const c of ["a", "b"]) {
    await store.appendChunk(a.id, offset, big(c));
    offset += big(c).length;
  }

  assert.equal((await migrate()).status, 0);
  const chunks = await firestore()
    .collection("logs")
    .doc(storageId(a.id))
    .collection("chunks")
    .get();
  for (const chunk of chunks.docs) {
    assert.equal(
      chunk.get("end"),
      Number(chunk.id) + chunk.get("n"),
      `chunk ${chunk.id}`,
    );
  }
  await assert.rejects(
    store.appendChunk(a.id, offset + 1, big("c")),
    store.Misplaced,
  );
  await store.appendChunk(a.id, offset, big("c"));
  assert.equal(
    (await store.readChunks(a.id, "")).text,
    a.chunks.join("") + big("a") + big("b") + big("c"),
  );
});

test(
  "the migration changes nothing with the wrong key, or none",
  {skip, timeout: 60_000},
  async () => {
    const {firestore} = await import("../lib/firebase.ts");
    const log = await oldLog();
    const before = (await firestore().collection("logs").listDocuments())
      .length;

    const wrong = await migrate([], {
      AGENT_GRAPH_ENCRYPTION_KEY: Buffer.alloc(32, 7).toString("base64url"),
    });
    assert.equal(wrong.status, 1);
    assert.match(
      wrong.stderr,
      /decrypts with this AGENT_GRAPH_ENCRYPTION_KEY: either it isn't the site's key/,
    );
    assert.equal(
      (await firestore().collection("logs").listDocuments()).length,
      before,
      "nothing was copied",
    );
    const wrongDelete = await migrate(["--delete-old"], {
      AGENT_GRAPH_ENCRYPTION_KEY: Buffer.alloc(32, 7).toString("base64url"),
    });
    assert.equal(wrongDelete.status, 1);
    assert.ok((await log.doc.get()).exists, "nothing was deleted");

    // Outside the emulator, the key must be given (the development one would do the same harm).
    const none = await migrate([], {
      AGENT_GRAPH_ENCRYPTION_KEY: undefined,
      FIRESTORE_EMULATOR_HOST: undefined,
    });
    assert.equal(none.status, 2);
    assert.match(none.stderr, /Set AGENT_GRAPH_ENCRYPTION_KEY/);

    assert.equal((await migrate()).status, 0);
    assert.equal((await migrate(["--delete-old"])).status, 0);
    assert.ok(await gone(log.doc));
  },
);

test(
  "a log the migration can't move is left as it was, and the rest are moved",
  {skip, timeout: 60_000},
  async () => {
    const {firestore} = await import("../lib/firebase.ts");
    const store = await import("../lib/store.ts");
    const good = await oldLog();
    const damaged = await oldLog({encryptAs: "ZZZZZZZZZZZZ"});
    const noSource = await oldLog({meta: {}});
    const orphan = await oldLog({parent: false, chunks: ['{"n":9}\n']});
    // The newest, so it's moved first: a chunk that fits under its old name
    // but not the (longer) new one, so writing its copy fails.
    const tooBig = await oldLog();
    await tooBig.doc
      .collection("chunks")
      .doc(store.chunkKey(tooBig.offset))
      .set({e: Buffer.alloc(1_048_485)});

    const copied = await migrate();
    assert.equal(copied.status, 1, "says some failed");
    assert.match(copied.stderr, new RegExp(`logs/${tooBig.id}:`));
    assert.match(copied.stderr, /3 logs failed/);
    assert.equal(
      await store.getMeta(tooBig.id),
      null,
      "a log whose copy failed doesn't show",
    );
    assert.match(copied.stderr, new RegExp(`logs/${damaged.id}:`));
    assert.match(
      copied.stderr,
      new RegExp(`logs/${noSource.id}: its metadata has no source`),
    );
    assert.ok(await store.getMeta(good.id), "the good one is moved");
    assert.equal(
      await store.getMeta(damaged.id),
      null,
      "a log that failed doesn't show",
    );
    assert.equal(await store.getMeta(noSource.id), null);
    assert.equal(
      (await store.readChunks(orphan.id, "")).text,
      orphan.chunks.join(""),
      "chunks without their log, too",
    );

    const deleted = await migrate(["--delete-old"]);
    assert.equal(deleted.status, 1);
    assert.ok(await gone(good.doc));
    assert.ok(await gone(orphan.doc));
    assert.ok((await damaged.doc.get()).exists, "kept");
    assert.ok((await noSource.doc.get()).exists, "kept");

    // Leave no old logs for the other tests' migrations.
    for (const log of [damaged, noSource, tooBig])
      await firestore().recursiveDelete(log.doc);
  },
);

test(
  "a log without chunks is deleted only once the key is proven",
  {skip, timeout: 60_000},
  async () => {
    const store = await import("../lib/store.ts");
    const wrongKey = {
      AGENT_GRAPH_ENCRYPTION_KEY: Buffer.alloc(32, 7).toString("base64url"),
    };
    // The only old log, and it has no chunks: nothing to check a key against.
    const empty = await oldLog({chunks: []});

    // Copied, and "checked", with the wrong key, whose tags match its copies.
    assert.equal((await migrate([], wrongKey)).status, 0);
    const wrong = await migrate(["--delete-old"], wrongKey);
    assert.equal(wrong.status, 1);
    assert.match(
      wrong.stderr,
      new RegExp(`logs/${empty.id}: nothing could check the key`),
    );
    assert.ok((await empty.doc.get()).exists, "kept");

    // Nor with the right one, until the log's copy has a chunk to prove it.
    assert.equal((await migrate()).status, 0);
    assert.equal((await migrate(["--delete-old"])).status, 1);
    assert.ok((await empty.doc.get()).exists, "kept");
    await store.appendChunk(empty.id, 0, '{"n":1}\n');
    const proven = await migrate(["--delete-old"]);
    assert.equal(proven.status, 0, proven.stderr);
    assert.ok(await gone(empty.doc));
    assert.equal((await store.readChunks(empty.id, "")).text, '{"n":1}\n');
  },
);

// R16: a stored chunk that can't be decrypted is refused like any other
// mismatch (the route answers 409, so the client stops), rather than an
// error (a 500, which the client would retry for good).
test(
  "a stored chunk that can't be decrypted isn't replaced",
  {skip, timeout: 60_000},
  async () => {
    const {storageId} = await import("../lib/encryption.ts");
    const {firestore} = await import("../lib/firebase.ts");
    const store = await import("../lib/store.ts");
    const id = newId();
    const text = '{"n":1}\n';
    await store.createLog(id, {source: "watch"}, text);
    await store.appendChunk(id, text.length, text);

    const chunk = firestore()
      .collection("logs")
      .doc(storageId(id))
      .collection("chunks")
      .doc(store.chunkKey(text.length));
    await chunk.update({e: Buffer.from("not a chunk")});
    await assert.rejects(
      store.appendChunk(id, text.length, text),
      store.ChunkTaken,
    );
  },
);

/** A random address, so tests (and runs against the same emulator) don't share counts. */
const anAddress = () =>
  `10.${[0, 0, 0].map(() => Math.floor(Math.random() * 256)).join(".")}`;

// R19: guesses made at once can't get past a limit (they're counted in a
// transaction, and too many at once to count are refused, not an error),
// and the limit lifts when the window ends.
test(
  "password guesses are counted once each, and the count ends with its window",
  {skip, timeout: 120_000},
  async () => {
    const {Timestamp} = await import("firebase-admin/firestore");
    const {UNLOCK_WINDOW_MS, UNLOCKS_PER_LOG} =
      await import("../lib/config.ts");
    const {storageId} = await import("../lib/encryption.ts");
    const {firestore} = await import("../lib/firebase.ts");
    const store = await import("../lib/store.ts");

    const id = newId();
    const burst = await Promise.all(
      Array.from({length: UNLOCKS_PER_LOG * 2}, (_, i) =>
        store.takeUnlockAttempt(id, anAddress()),
      ),
    );
    const through = results => results.filter(r => "reservation" in r).length;
    assert.ok(
      through(burst) <= UNLOCKS_PER_LOG,
      `${through(burst)} let through at once`,
    );
    for (const r of burst.filter(r => "wait" in r))
      assert.ok(r.wait > 0 && r.wait <= UNLOCK_WINDOW_MS / 1000);
    // One at a time, the rest of the limit, and no more.
    let total = through(burst);
    for (let i = 0; i < UNLOCKS_PER_LOG; i++)
      total += through([await store.takeUnlockAttempt(id, anAddress())]);
    assert.equal(total, UNLOCKS_PER_LOG, "exactly the limit let through");

    const bucket = firestore()
      .collection("unlock-attempts")
      .doc(`log-${storageId(id)}`);
    assert.equal(
      (await bucket.get()).get("n"),
      UNLOCKS_PER_LOG,
      "refused guesses aren't counted",
    );

    // Full for a while: a flood is refused by a plain read, without a
    // transaction to contend over.
    await bucket.update({at: Timestamp.fromMillis(Date.now() - 60_000)});
    const {Firestore} = await import("firebase-admin/firestore");
    const {runTransaction} = Firestore.prototype;
    let transactions = 0;
    Firestore.prototype.runTransaction = function (...args) {
      transactions++;
      return runTransaction.apply(this, args);
    };
    let flood;
    try {
      const flooder = anAddress();
      flood = await Promise.all(
        Array.from({length: 50}, () => store.takeUnlockAttempt(id, flooder)),
      );
    } finally {
      Firestore.prototype.runTransaction = runTransaction;
    }
    assert.ok(
      flood.every(r => "wait" in r && r.wait > 60),
      "all told to wait for the window",
    );
    assert.equal(transactions, 0, "no transactions");

    // The window ends: counting starts again.
    await bucket.update({
      since: Timestamp.fromMillis(Date.now() - UNLOCK_WINDOW_MS - 1000),
    });
    assert.ok(
      "reservation" in (await store.takeUnlockAttempt(id, anAddress())),
    );
    assert.equal((await bucket.get()).get("n"), 1);
  },
);

// R19: a right guess is given back to every bucket it was counted in, but
// not into a window that began since.
test("a right guess is given back", {skip, timeout: 60_000}, async () => {
  const {Timestamp} = await import("firebase-admin/firestore");
  const {UNLOCK_WINDOW_MS} = await import("../lib/config.ts");
  const store = await import("../lib/store.ts");

  const id = newId();
  const address = anAddress();
  const first = await store.takeUnlockAttempt(id, address);
  const second = await store.takeUnlockAttempt(id, address);
  assert.equal(
    first.reservation.length,
    3,
    "the log, the address, and the two together",
  );
  const counts = async () =>
    Promise.all(
      first.reservation.map(async ({ref}) => (await ref.get()).get("n")),
    );
  assert.deepEqual(await counts(), [2, 2, 2]);
  await store.giveBackUnlockAttempt(first.reservation);
  assert.deepEqual(await counts(), [1, 1, 1]);

  // A new window since: nothing to give back to.
  for (const {ref} of second.reservation) {
    await ref.update({
      since: Timestamp.fromMillis(Date.now() - UNLOCK_WINDOW_MS - 1000),
    });
  }
  const renewed = await store.takeUnlockAttempt(id, address);
  await store.giveBackUnlockAttempt(second.reservation);
  assert.deepEqual(
    await counts(),
    [1, 1, 1],
    "the new window's guess stays counted",
  );
  assert.equal(renewed.reservation.length, 3);

  // Without an address, only the log counts it.
  assert.equal(
    (await store.takeUnlockAttempt(newId(), null)).reservation.length,
    1,
  );
});

// R19: too many guesses at once to count is a short wait, not an error; and
// a log full only of guesses still being checked (right ones among them are
// about to be given back) is a short wait too, not the window's end.
test(
  "guesses that can't be counted just now wait a moment",
  {skip, timeout: 60_000},
  async () => {
    const {Firestore, Timestamp} = await import("firebase-admin/firestore");
    const {UNLOCK_BUSY_SECONDS, UNLOCKS_PER_LOG} =
      await import("../lib/config.ts");
    const {storageId} = await import("../lib/encryption.ts");
    const {firestore} = await import("../lib/firebase.ts");
    const store = await import("../lib/store.ts");

    const {runTransaction} = Firestore.prototype;
    Firestore.prototype.runTransaction = () =>
      Promise.reject(Object.assign(new Error("contention"), {code: 10}));
    try {
      assert.deepEqual(await store.takeUnlockAttempt(newId(), anAddress()), {
        wait: UNLOCK_BUSY_SECONDS,
      });
    } finally {
      Firestore.prototype.runTransaction = runTransaction;
    }

    const id = newId();
    for (let i = 0; i < UNLOCKS_PER_LOG; i++)
      assert.ok(
        "reservation" in (await store.takeUnlockAttempt(id, anAddress())),
      );
    assert.deepEqual(
      await store.takeUnlockAttempt(id, anAddress()),
      {wait: UNLOCK_BUSY_SECONDS},
      "just filled",
    );
    const bucket = firestore()
      .collection("unlock-attempts")
      .doc(`log-${storageId(id)}`);
    await bucket.update({at: Timestamp.fromMillis(Date.now() - 60_000)});
    const later = await store.takeUnlockAttempt(id, anAddress());
    assert.ok(later.wait > 60, `full a while: ${later.wait}s`);
  },
);

// R52: every scrypt run counts against its address (2000 a window): a
// password checked, right or wrong, and a new log's hashed. Checks at a log
// count against the log and address together too (200 a window), so one
// log can't use up the address's. Neither is given back.
test(
  "scrypt runs are counted per address, and per log and address",
  {skip, timeout: 60_000},
  async () => {
    const {Timestamp} = await import("firebase-admin/firestore");
    const {
      SCRYPT_CHECKS_PER_LOG_AND_ADDRESS,
      SCRYPT_RUNS_PER_ADDRESS,
      UNLOCK_WINDOW_MS,
    } = await import("../lib/config.ts");
    const {addressKey} = await import("../lib/crypto.ts");
    const {storageId} = await import("../lib/encryption.ts");
    const {firestore} = await import("../lib/firebase.ts");
    const store = await import("../lib/store.ts");

    const address = anAddress();
    const id = newId();
    const buckets = firestore().collection("unlock-attempts");
    const runs = buckets.doc(`scrypt-${addressKey(address)}`);
    const checks = buckets.doc(`checks-${addressKey(address, storageId(id))}`);
    const right = await store.takeUnlockAttempt(id, address);
    await store.giveBackUnlockAttempt(right.reservation);
    assert.equal(
      (await runs.get()).get("n"),
      1,
      "a right guess still ran scrypt",
    );
    assert.equal((await checks.get()).get("n"), 1, "at this log");
    assert.deepEqual(await store.takeScryptRun(address), {});
    assert.equal((await runs.get()).get("n"), 2);

    // A log's checks, full: only that log waits.
    const long = Timestamp.fromMillis(Date.now() - 60_000);
    await checks.update({n: SCRYPT_CHECKS_PER_LOG_AND_ADDRESS, at: long});
    const full = await store.takeUnlockAttempt(id, address);
    assert.ok(full.wait > 60, `this log waits for the window: ${full.wait}s`);
    assert.ok(
      "reservation" in (await store.takeUnlockAttempt(newId(), address)),
      "another log doesn't",
    );
    assert.deepEqual(
      await store.takeScryptRun(address),
      {},
      "nor does creating a log",
    );
    assert.ok(
      "reservation" in (await store.takeUnlockAttempt(id, anAddress())),
      "nor another address",
    );

    // The address's runs, full: everything from it waits.
    await runs.update({n: SCRYPT_RUNS_PER_ADDRESS, at: long});
    const created = await store.takeScryptRun(address);
    assert.ok(
      created.wait > 60,
      `a new log's password waits for the window: ${created.wait}s`,
    );
    const guess = await store.takeUnlockAttempt(newId(), address);
    assert.ok(guess.wait > 60, `so does a guess at any log: ${guess.wait}s`);
    assert.equal(
      (await runs.get()).get("n"),
      SCRYPT_RUNS_PER_ADDRESS,
      "refused runs aren't counted",
    );
    assert.deepEqual(
      await store.takeScryptRun(anAddress()),
      {},
      "other addresses aren't affected",
    );
    assert.deepEqual(
      await store.takeScryptRun(null),
      {},
      "without an address, not limited",
    );

    await runs.update({
      since: Timestamp.fromMillis(Date.now() - UNLOCK_WINDOW_MS - 1000),
    });
    assert.deepEqual(await store.takeScryptRun(address), {});
    assert.equal((await runs.get()).get("n"), 1);
  },
);

// Metadata can go (a log not in use is deleted): each instance's cache of it
// is kept only so long.
test(
  "cached metadata is read again after a while",
  {skip, timeout: 60_000},
  async () => {
    const store = await import("../lib/store.ts");
    const {firestore} = await import("../lib/firebase.ts");
    const {storageId} = await import("../lib/encryption.ts");
    const id = newId();
    await store.createLog(id, {source: "watch"}, "");
    assert.ok(await store.getMeta(id));
    await firestore().collection("logs").doc(storageId(id)).delete();
    assert.ok(await store.getMeta(id), "still cached");
    assert.equal(
      await store.getMeta(id, Date.now() + store.META_CACHE_MS + 1),
      null,
    );
  },
);

// /admin's live shares now (lib/analytics.ts): each says when events last
// came, to the minute, and an append writes it at most once a minute.
test(
  "a live share says when events last came, to the minute",
  {skip, timeout: 60_000},
  async () => {
    const store = await import("../lib/store.ts");
    const {activeWatches} = await import("../lib/analytics.ts");
    const {Timestamp} = await import("firebase-admin/firestore");
    const lastAt = async id =>
      (await copyOf(id)).get("lastAt")?.toMillis() ?? null;

    const anon = newId();
    await store.createLog(anon, {source: "paste"}, "A\n");
    await store.appendChunk(anon, 2, "B\n");
    assert.equal(await lastAt(anon), null, "only an account's live share");

    const id = newId();
    await store.createLog(id, {source: "watch", owner: "someone"}, "A\n");
    const made = await lastAt(id);
    assert.ok(made && Date.now() - made < 60_000);
    await store.appendChunk(id, 2, "B\n");
    assert.equal(await lastAt(id), made, "not again within the minute");

    // Quiet for ten minutes: not active.
    const before = await activeWatches();
    await (
      await copyOf(id)
    ).ref.update({
      lastAt: Timestamp.fromMillis(Date.now() - 10 * 60_000),
    });
    assert.equal(await activeWatches(), before - 1);
    // Events again: active, and it says so.
    await store.appendChunk(id, 4, "C\n");
    assert.ok(Date.now() - (await lastAt(id)) < 60_000);
    assert.equal(await activeWatches(), before);
  },
);
