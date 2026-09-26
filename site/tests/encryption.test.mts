// Unit tests for lib/encryption.ts:  npm run test:unit

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { test } from "node:test";

process.env.AGENT_GRAPH_ENCRYPTION_KEY = Buffer.alloc(32, 1).toString("base64url");
const { decryptChunk, encryptChunk, metaTag, storageId } = await import("../lib/encryption.ts");

const LOG = "AbCdEf123456";
const CHUNK = "000000000000000";
const TEXT = '{"v":1,"id":"01K","type":"status","data":{"summary":"héllo ✓ 修复"}}\n';

test("round-trips text, including non-ASCII", () => {
  assert.equal(decryptChunk(LOG, CHUNK, encryptChunk(LOG, CHUNK, TEXT)), TEXT);
  assert.equal(decryptChunk(LOG, CHUNK, encryptChunk(LOG, CHUNK, "")), "");
});

test("stores no plaintext, and never the same bytes twice", () => {
  const a = encryptChunk(LOG, CHUNK, TEXT);
  const b = encryptChunk(LOG, CHUNK, TEXT);
  assert.ok(!a.includes(Buffer.from("summary")));
  assert.notDeepEqual(a, b, "a fresh IV each time");
  assert.equal(a.length, 1 + 12 + Buffer.byteLength(TEXT) + 16);
});

test("detects any change to the stored bytes", () => {
  const stored = encryptChunk(LOG, CHUNK, TEXT);
  for (const i of [1, 13, stored.length - 1]) {
    const tampered = Buffer.from(stored);
    tampered[i] ^= 1;
    assert.throws(() => decryptChunk(LOG, CHUNK, tampered), `byte ${i}`);
  }
  assert.throws(() => decryptChunk(LOG, CHUNK, stored.subarray(0, 20)), "truncated");
  assert.throws(() => decryptChunk(LOG, CHUNK, Buffer.concat([Buffer.of(9), stored.subarray(1)])), "unknown version");
});

test("a chunk can't be moved to another place or another log", () => {
  const stored = encryptChunk(LOG, CHUNK, TEXT);
  assert.throws(() => decryptChunk(LOG, "000000000000100", stored), "another offset");
  assert.throws(() => decryptChunk("ZZZZZZZZZZZZ", CHUNK, stored), "another log");
});

test("another master key can't read it", () => {
  const stored = encryptChunk(LOG, CHUNK, TEXT).toString("base64");
  const other = spawnSync(
    process.execPath,
    [
      "--experimental-strip-types",
      "--no-warnings",
      "--input-type=module",
      "-e",
      `const { decryptChunk } = await import(${JSON.stringify(new URL("../lib/encryption.ts", import.meta.url).href)});
       decryptChunk(${JSON.stringify(LOG)}, ${JSON.stringify(CHUNK)}, Buffer.from(${JSON.stringify(stored)}, "base64"));`,
    ],
    { env: { ...process.env, AGENT_GRAPH_ENCRYPTION_KEY: Buffer.alloc(32, 2).toString("base64url") } },
  );
  assert.notEqual(other.status, 0);
});

test("rejects a master key of the wrong size", () => {
  const bad = spawnSync(
    process.execPath,
    [
      "--experimental-strip-types",
      "--no-warnings",
      "--input-type=module",
      "-e",
      `const { encryptChunk } = await import(${JSON.stringify(new URL("../lib/encryption.ts", import.meta.url).href)});
       encryptChunk("x", "0", "text");`,
    ],
    { env: { ...process.env, AGENT_GRAPH_ENCRYPTION_KEY: "too-short" }, encoding: "utf8" },
  );
  assert.notEqual(bad.status, 0);
  assert.match(bad.stderr, /must be 32 bytes/);
});

/** Runs `expr` against lib/encryption.ts under another master key; returns its JSON. */
function underAnotherKey(expr: string): unknown {
  const other = spawnSync(
    process.execPath,
    [
      "--experimental-strip-types",
      "--no-warnings",
      "--input-type=module",
      "-e",
      `const { metaTag, storageId } = await import(${JSON.stringify(new URL("../lib/encryption.ts", import.meta.url).href)});
       console.log(JSON.stringify(${expr}));`,
    ],
    { env: { ...process.env, AGENT_GRAPH_ENCRYPTION_KEY: Buffer.alloc(32, 2).toString("base64url") }, encoding: "utf8" },
  );
  assert.equal(other.status, 0, other.stderr);
  return JSON.parse(other.stdout);
}

// R18: where a log is stored can't be worked out from its id without the
// master key, and can't be turned back into the id.
test("a log's storage id is keyed", () => {
  const sid = storageId(LOG);
  assert.match(sid, /^[A-Za-z0-9_-]{43}$/);
  assert.equal(storageId(LOG), sid, "the same every time");
  assert.notEqual(storageId("ZZZZZZZZZZZZ"), sid);
  assert.notEqual(sid, createHash("sha256").update(LOG).digest("base64url"), "not an unkeyed hash");
  assert.notEqual(underAnotherKey(`storageId(${JSON.stringify(LOG)})`), sid, "another master key");
});

// R18: a log's metadata tag covers its id, source and password, and needs
// the master key.
test("a metadata tag covers the id, the source and the password", () => {
  const meta = { source: "watch", pw: "s1$salt$hash" };
  const tag = metaTag(LOG, meta);
  assert.match(tag, /^[A-Za-z0-9_-]{43}$/);
  assert.equal(metaTag(LOG, { ...meta }), tag);
  assert.notEqual(metaTag("ZZZZZZZZZZZZ", meta), tag, "another log");
  assert.notEqual(metaTag(LOG, { ...meta, source: "paste" }), tag, "another source");
  assert.notEqual(metaTag(LOG, { ...meta, pw: "s1$salt$other" }), tag, "another password");
  assert.notEqual(metaTag(LOG, { source: "watch" }), tag, "no password");
  assert.equal(metaTag(LOG, { source: "watch", pw: "" }), metaTag(LOG, { source: "watch" }), "an empty password is none");
  assert.notEqual(underAnotherKey(`metaTag(${JSON.stringify(LOG)}, ${JSON.stringify(meta)})`), tag, "another master key");
});

// R18: the storage id and the tags use keys of their own, so neither can
// stand in for the other.
test("storage ids and tags are keyed apart", () => {
  const meta = { source: "watch" };
  assert.notEqual(storageId(JSON.stringify(["meta/1", LOG, "watch", null])), metaTag(LOG, meta));
});
