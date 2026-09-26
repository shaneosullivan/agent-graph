// lib/store.ts against the Firestore emulator, directly:  npm run test:ci
// (which runs it inside the emulator, after the API tests).

import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { register } from "node:module";
import { test } from "node:test";

register("./resolve-ts.mjs", import.meta.url);

const skip = !process.env.FIRESTORE_EMULATOR_HOST && "needs the Firestore emulator";

/** Records every query's limit and every chunk fetched while `fn` runs. */
async function watchQueries(fn) {
  const { Query } = await import("firebase-admin/firestore");
  const seen = { limits: [], fetched: 0 };
  const { limit, get, stream } = Query.prototype;
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
    throw new Error("a stream isn't cancelled by leaving it, so it isn't bounded");
  };
  try {
    return { result: await fn(), ...seen };
  } finally {
    Object.assign(Query.prototype, { limit, get, stream });
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
  Array.from(randomBytes(12), (b) => "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"[b % 62]).join("");

// R17: a read holds a bounded number of chunks at once, and fetches a
// bounded number beyond those it uses, however big the chunks are. (Not
// just the response: the fetching is what passes through memory.)
test("a read fetches chunks a few at a time", { skip, timeout: 60_000 }, async () => {
  const { BYTES_PER_READ, CHUNKS_PER_QUERY, MAX_CHUNK_BYTES } = await import("../lib/config.ts");
  const store = await import("../lib/store.ts");

  const id = newId();
  const big = `${"x".repeat(MAX_CHUNK_BYTES - 1)}\n`;
  await store.createLog(id, { source: "watch" }, big);
  const count = CHUNKS_PER_QUERY + 8;
  for (let i = 1; i < count; i++) await store.appendChunk(id, i * MAX_CHUNK_BYTES, big);

  const { result: read, limits, fetched } = await watchQueries(() => store.readChunks(id, ""));
  assert.ok(read.more);
  const used = Buffer.byteLength(read.text) / MAX_CHUNK_BYTES;
  assert.equal(used, Math.ceil(BYTES_PER_READ / MAX_CHUNK_BYTES));
  assert.ok(limits.length > 0 && limits.every((n) => n <= CHUNKS_PER_QUERY), `limits: ${limits}`);
  assert.ok(fetched < used + CHUNKS_PER_QUERY, `${fetched} fetched for ${used} used`);

  // And paging from there gets the rest, in order.
  assert.equal(read.text + (await rest(store, id, read, count)), big.repeat(count));
});

// R17: a read of many small chunks takes several queries, each carrying on
// from the last, and stops at CHUNKS_PER_READ, saying there's more.
test("a read of many small chunks carries on across queries", { skip, timeout: 120_000 }, async () => {
  const { CHUNKS_PER_QUERY, CHUNKS_PER_READ } = await import("../lib/config.ts");
  const store = await import("../lib/store.ts");

  const id = newId();
  const count = CHUNKS_PER_READ + 50;
  const chunk = (i) => `{"n":${i}}\n`;
  await store.createLog(id, { source: "watch" }, chunk(0));
  let offset = Buffer.byteLength(chunk(0));
  for (let i = 1; i < count; i++) {
    await store.appendChunk(id, offset, chunk(i));
    offset += Buffer.byteLength(chunk(i));
  }
  const all = Array.from({ length: count }, (_, i) => chunk(i)).join("");
  const upTo = (n) => Array.from({ length: n }, (_, i) => chunk(i)).join("");

  const { result: first, limits } = await watchQueries(() => store.readChunks(id, ""));
  assert.equal(first.text, upTo(CHUNKS_PER_READ), "each chunk once, in order");
  assert.ok(first.more, "says there's more");
  assert.equal(limits.length, Math.ceil(CHUNKS_PER_READ / CHUNKS_PER_QUERY));
  assert.ok(limits.every((n) => n <= CHUNKS_PER_QUERY), `limits: ${limits}`);

  const second = await store.readChunks(id, first.last);
  assert.equal(first.text + second.text, all);
  assert.equal(second.more, false);
});
